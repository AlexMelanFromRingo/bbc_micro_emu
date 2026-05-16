//! Boots MOS+BASIC, types a BASIC expression via OSRDCH passthrough, and
//! verifies the answer renders on screen.

use std::path::PathBuf;

use bbc_micro_emu::{Machine, MachineConfig, MemoryConfig};

#[test]
#[ignore = "needs roms/os120.rom + roms/basic2.rom"]
fn basic_prints_expression_result() {
    let mos = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/os120.rom");
    let basic = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/basic2.rom");

    let mut mem = MemoryConfig {
        mos_rom_path: Some(mos),
        initial_bank: 15,
        ..MemoryConfig::default()
    };
    mem.rom_banks[15] = Some(basic);
    let mut machine = Machine::new(MachineConfig { memory: mem }).unwrap();

    // Boot to BASIC prompt.
    machine.run_for_cycles(5_000_000, u64::MAX).unwrap();
    let ram = machine.bus.memory.ram();
    assert_eq!(ram[0x7CC8], b'>', "BASIC prompt missing");

    // Type "PRINT 1+1" and a newline. BASIC should evaluate it and print "2".
    machine.type_string("PRINT 1+1\n");
    eprintln!("Queue len before run: {}", machine.typed_chars_len());
    // Sample BASIC PC during the run.
    let mut pcs = Vec::new();
    for _ in 0..10 {
        machine.run_for_cycles(500_000, u64::MAX).unwrap();
        pcs.push(machine.cpu.registers.pc);
    }
    eprintln!("PC samples (over 5M cycles): {:04X?}", pcs);
    eprintln!("Queue len after run: {}", machine.typed_chars_len());

    // The answer "2" should appear somewhere after the '>' prompt. Dump a few
    // rows for diagnostic visibility.
    let ram = machine.bus.memory.ram();
    eprintln!("Row 5: {:?}", String::from_utf8_lossy(&ram[0x7CC8..0x7CF0]));
    eprintln!("Row 6: {:?}", String::from_utf8_lossy(&ram[0x7CF0..0x7D18]));
    eprintln!("Row 7: {:?}", String::from_utf8_lossy(&ram[0x7D18..0x7D40]));
    eprintln!("Row 8: {:?}", String::from_utf8_lossy(&ram[0x7D40..0x7D68]));

    // We expect a '2' to appear in MODE 7 RAM somewhere after $7CC8.
    let has_two = ram[0x7CC8..0x7E00].contains(&b'2');
    assert!(has_two, "BASIC did not print '2' after `PRINT 1+1`");
}
