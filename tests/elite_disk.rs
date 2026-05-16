//! Diagnostic harness for booting Elite (1984) `.ssd` images.
//! Marked `#[ignore]` — needs roms/* and a disk image in disks/.
//!
//! Two images are tried:
//! - `disks/Elite.ssd` — the original Acornsoft ELITEBBC.SSD. Its disc title
//!   bytes have the high bit set (probably part of the original copy
//!   protection), which appears to confuse the DFS workspace.
//! - `disks/elite_jsbeeb.ssd` — Ian Bell's fixed-up Elite image bundled with
//!   jsbeeb. Standard ASCII title, boot option 3 (*EXEC).

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
    mem.rom_banks[14] = Some(root.join("roms/dfs090.rom"));
    mem.rom_banks[15] = Some(root.join("roms/basic2.rom"));
    Machine::new(MachineConfig { memory: mem }).unwrap()
}

fn mount(machine: &mut Machine, disk_name: &str) -> bool {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("disks")
        .join(disk_name);
    if !path.exists() {
        eprintln!("disk missing: {}", path.display());
        return false;
    }
    machine
        .bus
        .hardware
        .fdc
        .load_image(0, std::fs::read(&path).unwrap())
        .unwrap();
    true
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd"]
fn elite_help_prints_dfs_banner() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
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

fn run_with_disk(disk_name: &str, command: &str, label: &str) {
    let mut machine = build_machine();
    if !mount(&mut machine, disk_name) {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    eprintln!(
        "=== {label}: AFTER BOOT ({}) ===",
        machine.bus.hardware.access_summary()
    );
    machine.type_string(command);
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();
    eprintln!(
        "=== {label}: AFTER {command:?} ({}) ===",
        machine.bus.hardware.access_summary()
    );
    eprintln!(
        "PC=${:04X}  Video CR=${:02X}",
        machine.cpu.registers.pc, machine.bus.hardware.video_ula.control,
    );
    dump_mode7(&machine);
    let mut fb = Framebuffer::new();
    machine.render_into(&mut fb);
    let non_black = fb.pixels.iter().filter(|p| **p != 0).count();
    eprintln!("Framebuffer non-black pixels: {non_black}");
    let path = format!("/tmp/elite_{}.ppm", label.replace(' ', "_").to_lowercase());
    fb.save_ppm(std::path::Path::new(&path)).unwrap();
    eprintln!("screenshot: {path}");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd (Acornsoft original)"]
fn elite_acornsoft_run_boot_diagnostic() {
    run_with_disk("Elite.ssd", "*RUN !BOOT\n", "acornsoft");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd; just looks the file up in catalog"]
fn elite_info_diagnostic() {
    run_with_disk("Elite.ssd", "*INFO !BOOT\n", "acornsoft_info");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd; prints the catalogue"]
fn elite_cat_diagnostic() {
    run_with_disk("Elite.ssd", "*CAT\n", "acornsoft_cat");
}

#[test]
#[ignore = "needs roms/* + disks/elite_jsbeeb.ssd (Ian Bell fixed-up image)"]
fn elite_jsbeeb_exec_load_diagnostic() {
    run_with_disk("elite_jsbeeb.ssd", "*EXEC LOAD\n", "jsbeeb_exec");
}

#[test]
#[ignore = "needs roms/* + disks/elite_jsbeeb.ssd"]
fn elite_jsbeeb_run_eltcode_diagnostic() {
    run_with_disk("elite_jsbeeb.ssd", "*RUN EltCode\n", "jsbeeb_eltcode");
}
