//! Diagnostic harness for booting the real Elite (1984) `.ssd` image.
//! Marked `#[ignore]` — needs roms/ and disks/Elite.ssd populated locally.
//!
//! Current status: DFS receives the catalogue bytes correctly (verified via
//! `FDC_TRACE_BYTES=1`), but issuing any disk-touching command (*RUN, *CAT,
//! *.) sends DFS into a track-by-track sector-0 scan instead of seeking to
//! the requested file. The OSCLI/print path is fine — `*HELP` produces the
//! expected DFS 0.98 banner. Suspect either DFS service-call workspace
//! corruption or a missing FDC result-status nuance.

use std::path::PathBuf;

use bbc_micro_emu::{Framebuffer, Machine, MachineConfig, MemoryConfig};

fn dump_mode7(machine: &Machine) {
    let ram = machine.bus.memory.ram();
    for row in 0..25 {
        let addr = 0x7C00 + row * 40;
        let line: String = ram[addr..addr + 40]
            .iter()
            .map(|&b| {
                let c = b & 0x7F;
                if (0x20..0x7F).contains(&c) {
                    c as char
                } else {
                    '.'
                }
            })
            .collect();
        eprintln!("row {row:2}: {line:?}");
    }
}

fn build_machine() -> Machine {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut mem = MemoryConfig {
        mos_rom_path: Some(root.join("roms/os120.rom")),
        initial_bank: 15,
        ..MemoryConfig::default()
    };
    mem.rom_banks[14] = Some(root.join("roms/dfs098.rom"));
    mem.rom_banks[15] = Some(root.join("roms/basic2.rom"));
    Machine::new(MachineConfig { memory: mem }).unwrap()
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd"]
fn elite_help_prints_dfs_banner() {
    // Sanity: with the disk inserted but only *HELP issued, DFS must print
    // its banner and the FDC must stay idle.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk = root.join("disks/Elite.ssd");
    if !disk.exists() {
        panic!("Elite.ssd missing — extract ELITEBBC.SSD into disks/");
    }
    let mut machine = build_machine();
    machine
        .bus
        .hardware
        .fdc
        .load_image(0, std::fs::read(&disk).unwrap())
        .unwrap();
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*HELP\n");
    machine.run_for_cycles(20_000_000, u64::MAX).unwrap();

    let ram = machine.bus.memory.ram();
    let teletext: String = (0..25)
        .flat_map(|row| {
            ram[0x7C00 + row * 40..0x7C00 + row * 40 + 40]
                .iter()
                .map(|&b| (b & 0x7F) as char)
        })
        .collect();
    assert!(
        teletext.contains("DFS 0.98"),
        "DFS 0.98 banner missing from MODE 7 RAM after *HELP"
    );
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd; documents current Elite-disk WIP"]
fn elite_run_boot_diagnostic() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk = root.join("disks/Elite.ssd");
    if !disk.exists() {
        panic!("Elite.ssd missing — extract ELITEBBC.SSD into disks/");
    }
    let mut machine = build_machine();
    machine
        .bus
        .hardware
        .fdc
        .load_image(0, std::fs::read(&disk).unwrap())
        .unwrap();

    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    eprintln!(
        "=== AFTER BOOT ({}) ===",
        machine.bus.hardware.access_summary()
    );
    dump_mode7(&machine);

    machine.type_string("*RUN !BOOT\n");
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();
    eprintln!(
        "=== AFTER *RUN !BOOT, 300M cycles ({}) ===",
        machine.bus.hardware.access_summary()
    );
    eprintln!(
        "PC=${:04X}  Video CR=${:02X}  CRTC R12=${:02X} R13=${:02X}",
        machine.cpu.registers.pc,
        machine.bus.hardware.video_ula.control,
        machine.bus.hardware.crtc.reg(12),
        machine.bus.hardware.crtc.reg(13),
    );
    dump_mode7(&machine);

    let ram = machine.bus.memory.ram();
    eprintln!(
        "Non-zero bytes by region: $3000-$7FFF={} $5800-$7FFF={}",
        ram[0x3000..0x8000].iter().filter(|&&b| b != 0).count(),
        ram[0x5800..0x8000].iter().filter(|&&b| b != 0).count(),
    );
    let mut fb = Framebuffer::new();
    machine.render_into(&mut fb);
    let non_black = fb.pixels.iter().filter(|p| **p != 0).count();
    eprintln!("Framebuffer non-black pixels: {non_black}");
    fb.save_ppm(std::path::Path::new("/tmp/elite_after_run.ppm"))
        .unwrap();
}
