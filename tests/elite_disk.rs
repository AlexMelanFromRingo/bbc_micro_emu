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

use bbc_micro_emu::system_via::BbcKey;
use bbc_micro_emu::{Framebuffer, Machine, MachineConfig, MemoryConfig};

/// Briefly hold a real BBC key down so an in-game keyboard scan (Elite
/// reads PA via System VIA, not OSRDCH) can observe it. We hold for at
/// least one 50 Hz VSYNC period (≈320 000 cycles at 2 MHz) so Elite's
/// per-frame input read window is guaranteed to overlap the press.
fn tap_key(machine: &mut Machine, key: BbcKey) {
    machine.bus.hardware.system_via.set_key(key, true);
    machine.run_for_cycles(800_000, u64::MAX).unwrap();
    machine.bus.hardware.system_via.set_key(key, false);
    machine.run_for_cycles(200_000, u64::MAX).unwrap();
}

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
        teletext.contains("DFS 0.90")
            || teletext.contains("DFS 0.98")
            || teletext.contains("DFS 1."),
        "DFS banner missing from MODE 7 RAM after *HELP"
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
    run_with_disk("Elite.ssd", "*RUN $.!BOOT\n", "acornsoft");
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
#[ignore = "needs roms/* + disks/Elite.ssd; try explicit dir prefix"]
fn elite_info_with_dir_prefix() {
    run_with_disk("Elite.ssd", "*INFO $.!BOOT\n", "info_dir");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd; wildcard"]
fn elite_info_wildcard() {
    run_with_disk("Elite.ssd", "*INFO *\n", "info_wild");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd; try EXEC and LOAD too"]
fn elite_load_diagnostic() {
    run_with_disk("Elite.ssd", "*LOAD $.!BOOT 1900\n", "load_dir");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd; try Elite4 (BASIC source)"]
fn elite_chain_elite4_diagnostic() {
    run_with_disk("Elite.ssd", "CHAIN \"Elite4\"\n", "chain_elite4");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd"]
fn elite_help_then_info_diagnostic() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*HELP\n");
    machine.run_for_cycles(20_000_000, u64::MAX).unwrap();
    machine.type_string("*INFO !BOOT\n");
    machine.run_for_cycles(40_000_000, u64::MAX).unwrap();
    dump_mode7(&machine);
    let ram = machine.bus.memory.ram();
    // Search MODE 0/MODE 4 framebuffers for "!BOOT" output too.
    for region_start in [0x3000usize, 0x5800, 0x6000] {
        let region = &ram[region_start..0x8000];
        if region.windows(5).any(|w| w == b"!BOOT") {
            eprintln!("found !BOOT text in ${region_start:04X}..");
        }
    }
    // Dump MOS extended-vector page ($0200..$0300) for diagnostic.
    eprintln!("$0200-$0240: {:02X?}", &ram[0x0200..0x0240]);
    eprintln!("$0380-$0390: {:02X?}", &ram[0x0380..0x0390]);
    eprintln!("$2600-$2620: {:02X?}", &ram[0x2600..0x2620]);
    eprintln!("$25F0-$2610: {:02X?}", &ram[0x25F0..0x2610]);
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

#[test]
#[ignore = "needs roms/* + disks/Welcome.ssd — Acornsoft Welcome tape"]
fn other_game_welcome_disc() {
    run_with_disk("Welcome.ssd", "*EXEC content\n", "welcome");
}

#[test]
#[ignore = "needs roms/* + disks/frogman.ssd"]
fn other_game_frogman() {
    run_with_disk("frogman.ssd", "*EXEC !Boot\n", "frogman");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd — try every key looking for launch"]
fn elite_find_launch_key() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*RUN $.!BOOT\n");
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();

    let mut fb_baseline = Framebuffer::new();
    machine.render_into(&mut fb_baseline);

    let candidates: &[(&str, BbcKey)] = &[
        ("Tab", BbcKey::Tab),
        ("Return", BbcKey::Return),
        ("Space", BbcKey::Space),
        ("Escape", BbcKey::Escape),
        ("K0", BbcKey::K0),
        ("K1", BbcKey::K1),
        ("F0", BbcKey::F0),
        ("F9", BbcKey::F9),
        ("Slash", BbcKey::Slash),
    ];

    for (label, key) in candidates {
        machine.bus.hardware.system_via.set_key(*key, true);
        machine.run_for_cycles(1_500_000, u64::MAX).unwrap();
        machine.bus.hardware.system_via.set_key(*key, false);
        machine.run_for_cycles(15_000_000, u64::MAX).unwrap();
        let mut fb = Framebuffer::new();
        machine.render_into(&mut fb);
        let diff = fb
            .pixels
            .iter()
            .zip(fb_baseline.pixels.iter())
            .filter(|(a, b)| a != b)
            .count();
        let p = format!(
            "/tmp/elite_after_{}.ppm",
            label.to_lowercase().replace(['/', '!'], "_")
        );
        fb.save_ppm(std::path::Path::new(&p)).unwrap();
        eprintln!(
            "after {label:>8}: PC=${:04X}  CR=${:02X}  pixel-diff vs baseline = {diff}  → {p}",
            machine.cpu.registers.pc, machine.bus.hardware.video_ula.control,
        );
    }
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd — investigate what blocks the launch"]
fn elite_launch_pc_histogram() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*RUN $.!BOOT\n");
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();
    eprintln!(
        "=== AT DOCKED SCREEN ===  PC=${:04X}",
        machine.cpu.registers.pc
    );

    // Hold F0 for a full 2 seconds (4M cycles) — much longer than tap_key —
    // to give Elite multiple per-frame keyboard scans to definitively
    // observe a press → release transition.
    machine.bus.hardware.system_via.set_key(BbcKey::F0, true);
    machine.run_for_cycles(4_000_000, u64::MAX).unwrap();
    machine.bus.hardware.system_via.set_key(BbcKey::F0, false);
    eprintln!("=== F0 RELEASED ===  PC=${:04X}", machine.cpu.registers.pc);

    // Run for 5 sec of CPU time and build a PC histogram so we can see
    // where Elite is spinning.
    use std::collections::HashMap;
    let mut hist: HashMap<u16, u32> = HashMap::new();
    for _ in 0..1000 {
        machine.run_for_cycles(10_000, u64::MAX).unwrap();
        *hist.entry(machine.cpu.registers.pc).or_insert(0) += 1;
    }
    let mut top: Vec<_> = hist.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    eprintln!("Top PCs (5s after F0):");
    for (pc, n) in top.iter().take(15) {
        let bank = machine.bus.memory.selected_bank();
        eprintln!("  ${pc:04X}  {n:>4}  bank={bank}");
    }
    let mut fb = Framebuffer::new();
    machine.render_into(&mut fb);
    fb.save_ppm(std::path::Path::new("/tmp/elite_post_long_f0.ppm"))
        .unwrap();
    eprintln!(
        "final PC=${:04X}  CR=${:02X}",
        machine.cpu.registers.pc, machine.bus.hardware.video_ula.control,
    );
    let ram = machine.bus.memory.ram();
    eprintln!(
        "non-zero bytes in MODE 4/5 viewport ($5800..$7FFF) = {}",
        ram[0x5800..0x8000].iter().filter(|&&b| b != 0).count()
    );
    eprintln!(
        "non-zero bytes in MODE 0/1 viewport ($3000..$8000) = {}",
        ram[0x3000..0x8000].iter().filter(|&&b| b != 0).count()
    );
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd — Elite in deep space"]
fn elite_in_flight_long_capture() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*RUN $.!BOOT\n");
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();
    tap_key(&mut machine, BbcKey::F0);

    // Snapshot every ~0.5s of CPU time for the next ~10s and keep the
    // frame with the most non-black pixels (a heuristic for "Cobra is
    // actually showing 3D content in the viewport"). Throttle up
    // periodically too.
    let mut best_count = 0usize;
    let mut best_pc = 0u16;
    let mut best_cr = 0u8;
    for round in 0..20 {
        if round % 3 == 1 {
            tap_key(&mut machine, BbcKey::KeyS);
        }
        machine.run_for_cycles(20_000_000, u64::MAX).unwrap();
        let mut fb = Framebuffer::new();
        machine.render_into(&mut fb);
        let count = fb.pixels.iter().filter(|p| **p != 0).count();
        let path = format!("/tmp/elite_flight_{round:02}.ppm");
        fb.save_ppm(std::path::Path::new(&path)).unwrap();
        if count > best_count {
            best_count = count;
            best_pc = machine.cpu.registers.pc;
            best_cr = machine.bus.hardware.video_ula.control;
        }
        eprintln!(
            "round {round:2}: PC=${:04X} CR=${:02X} pixels={count}",
            machine.cpu.registers.pc, machine.bus.hardware.video_ula.control,
        );
    }
    eprintln!("best frame: pixels={best_count} PC=${best_pc:04X} CR=${best_cr:02X}");
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd — F0 launches Cobra Mk III"]
fn elite_launch_sequence_renders_3d_viewport() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*RUN $.!BOOT\n");
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();

    // F0 = Launch from Docked screen on real BBC Elite. Snapshot at
    // several points during the cobalt-flag → 3D-viewport transition
    // (the launch tunnel animation takes ~2-3 seconds of CPU time).
    tap_key(&mut machine, BbcKey::F0);
    for stage in 0..6 {
        machine.run_for_cycles(60_000_000, u64::MAX).unwrap();
        let mut fb = Framebuffer::new();
        machine.render_into(&mut fb);
        let p = format!("/tmp/elite_launch_{stage}.ppm");
        fb.save_ppm(std::path::Path::new(&p)).unwrap();
        eprintln!(
            "launch stage {stage}: PC=${:04X}  CR=${:02X}  R12=${:02X} R13=${:02X}  saved {p}",
            machine.cpu.registers.pc,
            machine.bus.hardware.video_ula.control,
            machine.bus.hardware.crtc.reg(12),
            machine.bus.hardware.crtc.reg(13),
        );
    }
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd"]
fn elite_sound_chip_is_programmed_during_boot() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*RUN $.!BOOT\n");
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();

    // Elite plays a brief docking beep / theme during boot. By the time the
    // Docked screen is up, the SN76489 should have been programmed by the
    // BBC's Sound 0..3 ROM routines (volume command + period bytes via the
    // System VIA / IC32 /SOUND_WE handshake).
    let mut snd = machine
        .bus
        .hardware
        .sound
        .lock()
        .expect("audio mutex poisoned");
    let any_voice_set = (0..3).any(|c| snd.channel_attenuation(c) != 0x0F);
    let any_period_set = (0..3).any(|c| snd.channel_period(c) > 1);
    eprintln!(
        "ch0={:>2} ch1={:>2} ch2={:>2} noise={:>2}  periods: {} {} {}",
        snd.channel_attenuation(0),
        snd.channel_attenuation(1),
        snd.channel_attenuation(2),
        snd.channel_attenuation(3),
        snd.channel_period(0),
        snd.channel_period(1),
        snd.channel_period(2),
    );
    assert!(
        any_voice_set || any_period_set,
        "expected Elite's boot/docking sound to leave SN76489 programmed"
    );

    // Dump 0.5 s of synthesised audio so we can listen to it manually.
    let out = std::path::PathBuf::from("/tmp/elite_docked.wav");
    snd.dump_wav(&out, 22_050, 0.5).unwrap();
    eprintln!("audio: {}", out.display());
}

#[test]
#[ignore = "needs roms/* + disks/Elite.ssd — drives the game further into menus"]
fn elite_play_a_few_keys() {
    let mut machine = build_machine();
    if !mount(&mut machine, "Elite.ssd") {
        return;
    }
    machine.run_for_cycles(12_000_000, u64::MAX).unwrap();
    machine.type_string("*RUN $.!BOOT\n");
    machine.run_for_cycles(300_000_000, u64::MAX).unwrap();

    // Elite reads the keyboard via direct VIA matrix scan, not via
    // OSRDCH. Use the real key API: hold each key for ~100 ms then
    // release, run a chunk of cycles for the menu logic to react, dump
    // a screenshot. Sequence: Space (continue from Docked screen),
    // 1 (Launch), 4 (Galactic Chart), 6 (Cobra Mk III), 7 (Inventory).
    let sequence = [
        ("after_boot", None),
        ("after_f0_launch", Some(BbcKey::F0)),
        ("after_f1_buy", Some(BbcKey::F1)),
        ("after_f8_chart", Some(BbcKey::F3)),
        ("after_escape", Some(BbcKey::Escape)),
    ];
    for (label, maybe_key) in sequence {
        if let Some(k) = maybe_key {
            tap_key(&mut machine, k);
            machine.run_for_cycles(30_000_000, u64::MAX).unwrap();
        }
        let mut fb = Framebuffer::new();
        machine.render_into(&mut fb);
        let path = format!("/tmp/elite_play_{label}.ppm");
        fb.save_ppm(std::path::Path::new(&path)).unwrap();
        eprintln!("screenshot: {path}  (PC=${:04X})", machine.cpu.registers.pc);
    }
}
