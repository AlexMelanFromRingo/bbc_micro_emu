//! Demo: load the real MOS ROM (so the renderer's font cache is correct BBC
//! glyphs), then hand-configure MODE 7 and poke "BBC MICRO" into teletext RAM.
//! Produces /tmp/mode7_real_font.ppm for visual inspection.

use std::path::PathBuf;

use bbc_micro_emu::{Framebuffer, Machine, MachineConfig, MemoryConfig};
use mos6502_emu::Bus;

#[test]
#[ignore = "needs roms/os120.rom; produces /tmp/mode7_real_font.ppm"]
fn mode7_with_mos_font() {
    let mos_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/os120.rom");
    if !mos_path.exists() {
        panic!(
            "fixture missing: {}\nRun scripts/fetch_roms.sh first.",
            mos_path.display()
        );
    }

    let mem = MemoryConfig {
        mos_rom_path: Some(mos_path),
        ..MemoryConfig::default()
    };
    let mut machine = Machine::new(MachineConfig { memory: mem }).unwrap();

    // Don't run any CPU instructions. Program CRTC + Video ULA directly via
    // the bus so we control exactly what's in screen RAM.
    let crtc_writes: [(u8, u8); 6] = [
        (1, 40),    // R1 horizontal displayed
        (6, 25),    // R6 vertical displayed
        (9, 18),    // R9 scanlines per char
        (12, 0x28), // R12 screen start hi
        (13, 0x00), // R13 screen start lo
        (0, 63),    // R0 horizontal total
    ];
    for (reg, val) in crtc_writes {
        machine.bus.write(0xFE00, reg);
        machine.bus.write(0xFE01, val);
    }
    // Video ULA control: teletext mode (bit 1 set)
    machine.bus.write(0xFE20, 0x02);

    // Poke text into teletext screen RAM. Row 12, col 14, "BBC MICRO".
    let msg = b"\x07BBC MICRO";
    let base = 0x7C00 + 12u16 * 40 + 14u16;
    for (i, byte) in msg.iter().enumerate() {
        machine.bus.write(base + i as u16, *byte);
    }
    // Row 14, col 12, "PHASE 3 COMPLETE".
    let msg = b"\x02PHASE 3 COMPLETE";
    let base = 0x7C00 + 14u16 * 40 + 11u16;
    for (i, byte) in msg.iter().enumerate() {
        machine.bus.write(base + i as u16, *byte);
    }

    let mut fb = Framebuffer::new();
    machine.render_into(&mut fb);
    fb.save_ppm(std::path::Path::new("/tmp/mode7_real_font.ppm"))
        .unwrap();

    // Sanity: lots of green and white pixels should appear at the message rows.
    let mut white_at_row12 = 0;
    let mut green_at_row14 = 0;
    for y in 240..260 {
        for x in 0..bbc_micro_emu::renderer::SCREEN_W {
            let p = fb.pixels[y * bbc_micro_emu::renderer::SCREEN_W + x];
            if p == 0x00FF_FFFF {
                white_at_row12 += 1;
            }
        }
    }
    for y in 280..300 {
        for x in 0..bbc_micro_emu::renderer::SCREEN_W {
            let p = fb.pixels[y * bbc_micro_emu::renderer::SCREEN_W + x];
            if p == 0x0000_FF00 {
                green_at_row14 += 1;
            }
        }
    }
    assert!(
        white_at_row12 > 100,
        "expected white text on row 12, got {white_at_row12} pixels"
    );
    assert!(
        green_at_row14 > 100,
        "expected green text on row 14, got {green_at_row14} pixels"
    );
}
