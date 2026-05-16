//! Dumps a snapshot of screen RAM after running MOS for some cycles, to see
//! what (if anything) MOS has written.

use std::path::PathBuf;

use bbc_micro_emu::{Machine, MachineConfig, MemoryConfig};

#[test]
#[ignore = "needs roms/os120.rom"]
fn inspect_screen_ram_after_boot() {
    let mos_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/os120.rom");
    let basic_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/basic2.rom");

    let mut mem = MemoryConfig {
        mos_rom_path: Some(mos_path),
        initial_bank: 15,
        ..MemoryConfig::default()
    };
    mem.rom_banks[15] = Some(basic_path);
    let mut machine = Machine::new(MachineConfig { memory: mem }).unwrap();

    // Run ~5 MOS frames (1 frame = 40K cycles, so 200K).
    machine.run_for_cycles(8_000_000, u64::MAX).unwrap();

    eprintln!(
        "PC=${:04X}, CR=${:02X}",
        machine.cpu.registers.pc, machine.bus.hardware.video_ula.control
    );
    eprintln!(
        "CRTC start=${:04X}",
        machine.bus.hardware.crtc.display_start_crtc_addr()
    );
    eprintln!(
        "IC32=${:02X}, IFR=${:02X}, IER=${:02X}",
        machine.bus.hardware.system_via.ic32,
        machine.bus.hardware.system_via.via.ifr,
        machine.bus.hardware.system_via.via.ier
    );
    eprintln!(
        "bank={}, A=${:02X} X=${:02X} Y=${:02X} P=${:02X} SP=${:02X}",
        machine.bus.memory.selected_bank(),
        machine.cpu.registers.a,
        machine.cpu.registers.x,
        machine.cpu.registers.y,
        machine.cpu.registers.status,
        machine.cpu.registers.sp,
    );
    eprintln!(
        "T1 latch=${:04X} counter=${:04X} ACR=${:02X} PCR=${:02X}",
        machine.bus.hardware.system_via.via.t1_latch,
        machine.bus.hardware.system_via.via.t1_counter,
        machine.bus.hardware.system_via.via.acr,
        machine.bus.hardware.system_via.via.pcr,
    );
    // What's the ROM type byte ($8006) for each bank?
    let mut banks_present = vec![];
    for b in 0..16 {
        if machine.bus.memory.bank_is_present(b) {
            banks_present.push(b);
        }
    }
    eprintln!("ROMs present in banks: {:?}", banks_present);

    // Stack — last 8 bytes pushed.
    let stack_bytes: Vec<u8> = (0xF8..=0xFFu16)
        .map(|i| machine.bus.memory.ram()[0x0100 + i as usize])
        .collect();
    eprintln!("Stack $01F8..$01FF: {:02X?}", stack_bytes);

    // Look for the "BBC Computer 32K" string anywhere in RAM.
    let ram = machine.bus.memory.ram();
    let needle = b"BBC Computer";
    let mut found_at = vec![];
    for i in 0..ram.len().saturating_sub(needle.len()) {
        if &ram[i..i + needle.len()] == needle {
            found_at.push(i);
        }
    }
    eprintln!("'BBC Computer' found at: {:04X?}", found_at);

    // Dump teletext RAM ($7C00 + 12*40 .. + 14*40)
    eprintln!("$7C00 (first 40 bytes): {:02X?}", &ram[0x7C00..0x7C00 + 40]);
    eprintln!("$7C28 (row 1):          {:02X?}", &ram[0x7C28..0x7C28 + 40]);
    eprintln!("$7E70 (row 19):         {:02X?}", &ram[0x7E70..0x7E70 + 40]);

    // Also dump MODE 4/0 area
    eprintln!("$3000 (first 16): {:02X?}", &ram[0x3000..0x3010]);
    eprintln!("$5800 (first 16): {:02X?}", &ram[0x5800..0x5810]);

    // Count non-zero bytes in each candidate screen region
    let nz = |start: usize, len: usize| ram[start..start + len].iter().filter(|b| **b != 0).count();
    eprintln!("non-zero bytes per region:");
    eprintln!(
        "  $3000..$8000 ({:5} bytes) → {}",
        0x5000,
        nz(0x3000, 0x5000)
    );
    eprintln!(
        "  $5800..$8000 ({:5} bytes) → {}",
        0x2800,
        nz(0x5800, 0x2800)
    );
    eprintln!(
        "  $7C00..$8000 ({:5} bytes) → {}",
        0x0400,
        nz(0x7C00, 0x0400)
    );
}
