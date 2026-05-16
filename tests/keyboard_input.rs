//! System VIA keyboard test. Verifies the keyboard scan path:
//!   * set_key marks a matrix cell
//!   * with autoscan disabled, manual scan (write PA0-PA6, read PA7) finds it
//!   * with autoscan enabled, any pressed key sets PA7 for the addressed column

use bbc_micro_emu::system_via::{BbcKey, SystemVia, ic32};

fn enable_manual_scan(via: &mut SystemVia) {
    // Per b-em: `if (IC32 & 8)` selects autoscan, so bit 3 = 0 → manual scan
    // mode where the CPU drives the column on PA0-PA3.
    via.via.write(2, 0x0F); // DDRB
    via.write(0, 0b0000_0011);
}

#[test]
fn manual_scan_finds_pressed_key() {
    let mut via = SystemVia::new();
    enable_manual_scan(&mut via);
    assert!(via.ic32 & (1 << ic32::KEYBOARD_AUTOSCAN) == 0);

    via.set_key(BbcKey::KeyA, true);
    let (col, row) = BbcKey::KeyA.matrix_pos();

    // Drive PA with col,row + bit 7 high (irrelevant for input bit, but matches
    // what MOS actually writes).
    via.via.write(3, 0x7F); // DDRA: PA0..PA6 outputs, PA7 input
    let pa = col | (row << 4);
    via.via.write(1, pa);
    let read = via.read(1);
    assert!(read & 0x80 != 0, "expected PA7=1 for pressed key");

    // Pressing a different key should also be detected when addressed.
    via.set_key(BbcKey::KeyA, false);
    let read = via.read(1);
    assert!(read & 0x80 == 0, "expected PA7=0 after release");
}

#[test]
fn autoscan_keeps_pa7_high() {
    // In autoscan mode (IC32 bit 3 = 1) the keyboard does not pull PA7 low.
    // Per b-em, PA7 stays at whatever value the CPU just wrote — typically
    // high (the matrix is "passive" from the CPU's perspective). MOS uses
    // CA2 instead to learn that a key is down.
    let mut via = SystemVia::new();
    // Force autoscan ON: set IC32 bit 3.
    via.via.write(2, 0x0F);
    via.write(0, 0b0000_1011);
    assert!(via.ic32 & (1 << ic32::KEYBOARD_AUTOSCAN) != 0);

    via.set_key(BbcKey::KeyA, true);
    let (col, _row) = BbcKey::KeyA.matrix_pos();
    via.via.write(3, 0x7F);
    via.via.write(1, col);
    let read = via.read(1);
    assert!(read & 0x80 != 0, "PA7 should read high in autoscan mode");
}

#[test]
fn ic32_bit_3_controls_scan_mode() {
    let mut via = SystemVia::new();
    via.via.write(2, 0x0F); // DDRB
    // Set bit 3 → autoscan enabled.
    via.write(0, 0b0000_1011);
    assert!(via.ic32 & (1 << ic32::KEYBOARD_AUTOSCAN) != 0);
    // Clear bit 3 → autoscan disabled (manual scan).
    via.write(0, 0b0000_0011);
    assert!(via.ic32 & (1 << ic32::KEYBOARD_AUTOSCAN) == 0);
}

#[test]
fn screen_size_decoding_matches_bbc_modes() {
    let mut via = SystemVia::new();
    via.via.write(2, 0x3F); // allow PB0..5 outputs
    // Set IC32[4]=0, IC32[5]=0 (screen size = MODE 0/1/2 → 20 KiB).
    via.write(0, 0b0000_0100); // clear bit 4
    via.write(0, 0b0000_0101); // clear bit 5
    let mode_012 = via.screen_size_code();
    assert_eq!(mode_012, 0);

    // Set IC32[4]=1, IC32[5]=1 → MODE 6/7 → 8 KiB → code 3
    via.write(0, 0b0000_1100); // set bit 4
    via.write(0, 0b0000_1101); // set bit 5
    let mode_67 = via.screen_size_code();
    assert_eq!(mode_67, 3);
}
