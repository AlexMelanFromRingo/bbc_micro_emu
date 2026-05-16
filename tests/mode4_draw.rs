//! Verifies that BASIC's MODE 4 + DRAW writes pixels into screen RAM, and
//! that the renderer surfaces them. Helps catch screen-size / CRTC start
//! mismatches.

use std::path::PathBuf;

use bbc_micro_emu::{Framebuffer, Machine, MachineConfig, MemoryConfig};

#[test]
#[ignore = "needs roms/os120.rom + roms/basic2.rom"]
fn mode4_draw_paints_pixels() {
    let mos = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/os120.rom");
    let basic = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/basic2.rom");
    let mut mem = MemoryConfig {
        mos_rom_path: Some(mos),
        initial_bank: 15,
        ..MemoryConfig::default()
    };
    mem.rom_banks[15] = Some(basic);
    let mut machine = Machine::new(MachineConfig { memory: mem }).unwrap();

    machine.run_for_cycles(5_000_000, u64::MAX).unwrap();
    machine.type_string("MODE 0\nPRINT\"X\"\n");
    machine.run_for_cycles(20_000_000, u64::MAX).unwrap();
    eprintln!(
        "After MODE 0: palette={:?}",
        machine.bus.hardware.video_ula.palette
    );

    let ram = machine.bus.memory.ram();
    // Count non-zero bytes in candidate screen regions.
    let nz = |range: std::ops::Range<usize>| ram[range].iter().filter(|b| **b != 0).count();
    eprintln!(
        "Non-zero in $3000..$8000 (modes 0-7): {}",
        nz(0x3000..0x8000)
    );
    eprintln!(
        "Non-zero in $5800..$8000 (modes 4-7): {}",
        nz(0x5800..0x8000)
    );
    eprintln!(
        "Non-zero in $6000..$8000 (modes 6-7): {}",
        nz(0x6000..0x8000)
    );
    eprintln!(
        "Non-zero in $7C00..$8000 (mode 7):    {}",
        nz(0x7C00..0x8000)
    );
    eprintln!(
        "CRTC: start=${:04X} R6={} R9={} R12=${:02X} R13=${:02X}",
        machine.bus.hardware.crtc.display_start_crtc_addr(),
        machine.bus.hardware.crtc.reg(6),
        machine.bus.hardware.crtc.reg(9),
        machine.bus.hardware.crtc.reg(12),
        machine.bus.hardware.crtc.reg(13),
    );
    eprintln!(
        "IC32=${:02X}, screen_size_code={}, Video ULA CR=${:02X}, bpp={}, teletext={}",
        machine.bus.hardware.system_via.ic32,
        machine.bus.hardware.video_ula.screen_size_code,
        machine.bus.hardware.video_ula.control,
        machine.bus.hardware.video_ula.bits_per_pixel(),
        machine.bus.hardware.video_ula.teletext_mode(),
    );

    // Dump first non-zero region
    let ram = machine.bus.memory.ram();
    for i in 0x5800..0x8000 {
        if ram[i] != 0 {
            eprintln!("First non-zero in screen RAM: ${:04X} = ${:02X}", i, ram[i]);
            eprintln!(
                "Context: {:02X?}",
                &ram[i.saturating_sub(8)..(i + 16).min(0x8000)]
            );
            break;
        }
    }
    // Palette
    eprintln!("Palette: {:?}", machine.bus.hardware.video_ula.palette);

    // Render and count non-black pixels.
    let mut fb = Framebuffer::new();
    machine.render_into(&mut fb);
    let non_black = fb.pixels.iter().filter(|p| **p != 0).count();
    eprintln!("Framebuffer non-black pixels: {non_black}");
    fb.save_ppm(std::path::Path::new("/tmp/mode4_draw_test.ppm"))
        .unwrap();

    assert!(
        non_black > 100,
        "MODE 4 DRAW did not produce visible pixels"
    );
}
