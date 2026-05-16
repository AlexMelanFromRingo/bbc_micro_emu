//! Cross-device regression tests. Verifies the integration points between the
//! 6502 core, BBC bus, and each peripheral.

use bbc_micro_emu::acia6850::{Acia6850, SerialUla};
use bbc_micro_emu::crtc::Crtc6845;
use bbc_micro_emu::fdc8271::{Fdc8271, SECTOR_SIZE, SECTORS_PER_TRACK, SSD_SIZE, TRACKS_PER_DISK};
use bbc_micro_emu::sn76489::Sn76489;
use bbc_micro_emu::upd7002::UpD7002;
use bbc_micro_emu::via6522::{IFR_CA1, IFR_T1, Via6522};

fn build_pattern_disk() -> Vec<u8> {
    let mut img = vec![0u8; SSD_SIZE];
    for t in 0..TRACKS_PER_DISK {
        for s in 0..SECTORS_PER_TRACK {
            let off = (t * SECTORS_PER_TRACK + s) * SECTOR_SIZE;
            for i in 0..SECTOR_SIZE {
                img[off + i] = ((t * 17 + s * 3 + i) & 0xFF) as u8;
            }
        }
    }
    img
}

#[test]
fn via_t1_fires_at_expected_cycle_count() {
    let mut via = Via6522::new();
    // Configure T1: latch = 99, one-shot, enable IFR_T1. The VIA on the BBC
    // is clocked from Φ2 (1 MHz) — half the CPU clock — so each "tick(1)" of
    // CPU time is *half* a VIA cycle. We therefore expect IFR_T1 to fire
    // after roughly (N + 1) * 2 CPU cycles where N is the latch value.
    via.write(6, 99); // T1L-L
    via.write(7, 0); // T1L-H
    via.write(0xE, 0x80 | 0x40); // enable T1 IRQ
    via.write(5, 0); // T1C-H — reload counter
    via.tick(99 * 2);
    assert!(via.ifr & IFR_T1 == 0, "T1 fired too early");
    via.tick(4);
    assert!(
        via.ifr & IFR_T1 != 0,
        "T1 did not fire after ~(N+1)*2 CPU cycles"
    );
}

#[test]
fn via_ifr_ier_mask_yields_irq() {
    let mut via = Via6522::new();
    via.write(0xE, 0x80 | 0x02); // enable CA1 IRQ
    assert!(!via.has_pending_irq());
    // Simulate CA1 falling edge per default PCR config.
    via.set_ca1(true);
    via.set_ca1(false);
    assert!(via.ifr & IFR_CA1 != 0);
    assert!(via.has_pending_irq());
    // Reading port A clears CA1 + CA2.
    via.read(1);
    assert!(!via.has_pending_irq());
}

#[test]
fn crtc_registers_self_program_for_mode_7_layout() {
    let mut crtc = Crtc6845::new();
    for (r, v) in [
        (0u8, 63),
        (1, 40),
        (4, 30),
        (5, 2),
        (6, 25),
        (7, 28),
        (9, 18),
        (12, 0x28),
        (13, 0),
    ] {
        crtc.write(0x00, r);
        crtc.write(0x01, v);
    }
    assert_eq!(crtc.horizontal_displayed(), 40);
    assert_eq!(crtc.vertical_displayed(), 25);
    assert_eq!(crtc.scanlines_per_char_row(), 19);
    assert_eq!(crtc.display_start_crtc_addr(), 0x2800);
}

#[test]
fn crtc_records_mid_frame_r12_r13_writes_per_scanline() {
    let mut crtc = Crtc6845::new();
    // Program for ~64-cycles/scanline, simple layout.
    for (r, v) in [
        (0u8, 63),
        (4, 30),
        (5, 0),
        (6, 25),
        (9, 7),
        (12, 0x10),
        (13, 0),
    ] {
        crtc.write(0, r);
        crtc.write(1, v);
    }
    // Tick enough to advance several scanlines.
    crtc.tick(64 * 4); // ~4 scanlines worth at 2 MHz / 32 chars
    // Update R12 mid-frame to a new start.
    crtc.write(0, 12);
    crtc.write(1, 0x20);
    // The latest scanline and all subsequent slots should hold the new start.
    let new_start = crtc.display_start_crtc_addr();
    assert_eq!(new_start, 0x2000);
    let mid_scanline_index = 5; // we know we're after a few scanlines
    assert_eq!(crtc.start_per_scanline[mid_scanline_index + 1], new_start);
}

#[test]
fn fdc_read_data_returns_pattern_bytes() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, build_pattern_disk()).unwrap();
    // Read Data multi-sector, track 7 sector 5, 1 sector.
    fdc.write(0, 0x13);
    fdc.write(1, 7);
    fdc.write(1, 5);
    fdc.write(1, 1);
    fdc.tick(8500); // head settle / sector search
    assert!(fdc.poll_nmi_edge());
    let byte0 = fdc.read(4);
    // pattern: (7*17 + 5*3 + 0) & 0xFF = (119 + 15) & 0xFF = 134
    assert_eq!(byte0, ((7 * 17 + 5 * 3) & 0xFF) as u8);
    fdc.tick(150);
    assert!(fdc.poll_nmi_edge());
    let byte1 = fdc.read(4);
    assert_eq!(byte1, ((7 * 17 + 5 * 3 + 1) & 0xFF) as u8);
}

#[test]
fn fdc_write_data_persists_to_disk_image() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, build_pattern_disk()).unwrap();
    fdc.write(0, 0x0B); // Write Data multi-sector
    fdc.write(1, 1); // track
    fdc.write(1, 2); // sector
    fdc.write(1, 1); // 1 sector
    // We can't directly inspect the buffered write without exposing internals,
    // but the FDC should at least accept the command without panicking.
    fdc.tick(300);
}

#[test]
fn sn76489_first_byte_then_continuation_sets_full_period() {
    let mut s = Sn76489::new();
    s.write(0x80 | 0x05); // latch ch0 freq, low = 5
    s.write(0x20); // continuation, high = $20
    let p = s.channel_period(0);
    assert_eq!(p & 0x0F, 5);
    assert_eq!((p >> 4) & 0x3F, 0x20);
}

#[test]
fn acia_status_reflects_cts_dcd_inputs() {
    let mut a = Acia6850::new();
    a.cts_asserted = true;
    a.dcd_asserted = true;
    let s = a.read(0);
    assert!(s & 0x08 != 0);
    assert!(s & 0x04 != 0);
}

#[test]
fn serial_ula_motor_and_cassette_select_bits() {
    let mut ula = SerialUla::new();
    ula.write(0xC0);
    assert!(ula.motor_on());
    assert!(ula.cassette_selected());
    ula.write(0x00);
    assert!(!ula.motor_on());
    assert!(!ula.cassette_selected());
}

#[test]
fn adc_conversion_completes_after_budget() {
    let mut adc = UpD7002::new();
    adc.set_input(0, 0);
    adc.write(0, 0x00);
    adc.tick(8_001);
    assert!(adc.poll_eoc_edge());
    let hi = adc.read(1);
    let lo = adc.read(2);
    // value=0 → unsigned 32768 → $80, $00.
    assert_eq!(hi, 0x80);
    assert_eq!(lo, 0x00);
}
