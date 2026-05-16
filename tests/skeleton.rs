//! Phase 2 smoke tests: address-space dispatch and paged ROM switching.

use bbc_micro_emu::{Machine, MachineConfig, Memory, MemoryConfig, RomBank};
use mos6502_emu::{Bus, MemoryView};

fn build_synthetic_mos() -> Vec<u8> {
    // Minimal 16 KiB MOS ROM:
    //   reset vector at $FFFC -> $C000
    //   $C000: STA $FE30   ; switch to bank 1 (4 cycles + bus write)
    //          LDA $8000   ; read from sideways bank
    //          STA $0200   ; store to RAM
    //          STA $FE40   ; touch System VIA (logged)
    //          JMP *       ; halt loop
    let mut rom = vec![0u8; 0x4000];
    let program: [u8; 13] = [
        0xA9, 0x01, // LDA #$01
        0x8D, 0x30, 0xFE, // STA $FE30  (paged ROM select = bank 1)
        0xAD, 0x00, 0x80, // LDA $8000  (read from selected bank)
        0x8D, 0x00, 0x02, // STA $0200
        0x4C, 0x0B, // JMP $C00B (effective $C00B = JMP to itself once we patch)
    ];
    // Program at $C000:
    rom[0..program.len()].copy_from_slice(&program);
    // Make the JMP target = $C00B (the JMP itself) to create a self-loop.
    rom[0x0B] = 0x4C;
    rom[0x0C] = 0x0B;
    rom[0x0D] = 0xC0;

    // Reset vector at $FFFC = $C000
    rom[0x3FFC] = 0x00;
    rom[0x3FFD] = 0xC0;
    rom
}

fn build_marker_rom(marker: u8) -> Vec<u8> {
    let mut rom = vec![0u8; 0x4000];
    rom[0] = marker;
    rom
}

#[test]
fn machine_runs_without_real_roms_and_does_not_panic() {
    let machine = Machine::new(MachineConfig::default()).unwrap();
    assert_eq!(machine.cpu.cycles, 7); // reset takes 7 cycles
}

#[test]
fn synthetic_mos_program_executes_and_uses_paged_rom() {
    let mem = MemoryConfig {
        mos_rom_bytes: Some(build_synthetic_mos()),
        ..MemoryConfig::default()
    };
    // Pre-install bank 1 with a marker byte.
    let machine_config = MachineConfig { memory: mem };
    let mut machine = Machine::new(machine_config).unwrap();
    machine
        .bus
        .memory
        .install_bank(1, RomBank::from_bytes(&build_marker_rom(0xAB)).unwrap())
        .unwrap();
    assert_eq!(machine.cpu.registers.pc, 0xC000);

    let report = machine.run_for_cycles(10_000, 10_000).unwrap();
    assert!(report.instructions > 0);

    // RAM at $0200 should hold the marker from bank 1.
    assert_eq!(machine.bus.peek(0x0200), 0xAB);
    // The ROM-select latch must have routed the write.
    assert_eq!(machine.bus.memory.selected_bank(), 1);
    // System VIA region was *not* touched here; touch it via direct write to confirm dispatcher.
    machine.bus.write(0xFE40, 0x55);
    let s = machine.bus.hardware.access_summary();
    assert!(
        s.contains("SysVIA"),
        "expected SysVIA in access summary, got `{s}`"
    );
}

#[test]
fn sheila_dispatch_routes_to_correct_device() {
    use bbc_micro_emu::SheilaDevice;
    assert_eq!(SheilaDevice::from_addr(0xFE00), SheilaDevice::Crtc);
    assert_eq!(SheilaDevice::from_addr(0xFE07), SheilaDevice::Crtc);
    assert_eq!(SheilaDevice::from_addr(0xFE08), SheilaDevice::Acia);
    assert_eq!(SheilaDevice::from_addr(0xFE20), SheilaDevice::VideoUla);
    assert_eq!(SheilaDevice::from_addr(0xFE30), SheilaDevice::RomSelect);
    assert_eq!(SheilaDevice::from_addr(0xFE40), SheilaDevice::SystemVia);
    assert_eq!(SheilaDevice::from_addr(0xFE5F), SheilaDevice::SystemVia);
    assert_eq!(SheilaDevice::from_addr(0xFE60), SheilaDevice::UserVia);
    assert_eq!(SheilaDevice::from_addr(0xFE80), SheilaDevice::Fdc);
    assert_eq!(SheilaDevice::from_addr(0xFEC0), SheilaDevice::Adc);
    assert_eq!(SheilaDevice::from_addr(0xFEE0), SheilaDevice::Tube);
}

#[test]
fn memory_read_paths_cover_all_regions() {
    let mem_cfg = MemoryConfig {
        mos_rom_bytes: Some({
            let mut r = vec![0u8; 0x4000];
            r[0] = 0xC0; // first byte of MOS
            r[0x3FFF] = 0xFF; // last byte of MOS at $FFFF
            r
        }),
        ..MemoryConfig::default()
    };
    let mut memory = Memory::new(mem_cfg).unwrap();
    memory
        .install_bank(0, RomBank::from_bytes(&build_marker_rom(0x80)).unwrap())
        .unwrap();
    memory.ram_mut()[0x100] = 0x42;

    assert_eq!(memory.read(0x0100), 0x42);
    assert_eq!(memory.read(0x8000), 0x80);
    assert_eq!(memory.read(0xC000), 0xC0);
    assert_eq!(memory.read(0xFFFF), 0xFF);
}
