//! End-to-end smoke test: boots MOS 1.20 + BASIC II and verifies that the
//! BASIC prompt appears in MODE 7 screen RAM.

use std::path::PathBuf;

use bbc_micro_emu::{Machine, MachineConfig, MemoryConfig};

#[test]
#[ignore = "needs roms/os120.rom + roms/basic2.rom"]
fn boots_to_basic_prompt() {
    let mos = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/os120.rom");
    let basic = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/basic2.rom");

    let mut mem = MemoryConfig {
        mos_rom_path: Some(mos),
        initial_bank: 15,
        ..MemoryConfig::default()
    };
    mem.rom_banks[15] = Some(basic);
    let mut machine = Machine::new(MachineConfig { memory: mem }).unwrap();

    // 5M cycles is enough for MOS to print "BBC Computer 32K" / "BASIC" and
    // hand off to BASIC II which then prints its '>' prompt.
    machine.run_for_cycles(5_000_000, u64::MAX).unwrap();

    let ram = machine.bus.memory.ram();
    // Row 1 of MODE 7 screen RAM ($7C28) should contain the BBC banner.
    let banner = &ram[0x7C28..0x7C28 + 16];
    assert_eq!(
        banner,
        b"BBC Computer 32K",
        "expected 'BBC Computer 32K' at row 1, got {:?}",
        String::from_utf8_lossy(banner)
    );
    // Row 3 of MODE 7 ($7C78) holds the language ROM name.
    assert_eq!(
        &ram[0x7C78..0x7C78 + 5],
        b"BASIC",
        "expected language ROM name 'BASIC'"
    );
    // Row 5 ($7CC8) holds the BASIC '>' prompt.
    assert_eq!(ram[0x7CC8], b'>', "expected '>' prompt at row 5");
}
