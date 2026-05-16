//! CLI entry point. Default mode is windowed (opens a 640×512 winit window).
//! `--headless N` runs for N cycles, dumps a PPM screenshot and a SHEILA report.

use std::path::PathBuf;
use std::process::ExitCode;

use bbc_micro_emu::display::DisplayApp;
use bbc_micro_emu::{Framebuffer, Machine, MachineConfig, MemoryConfig};

fn print_usage() {
    eprintln!(
        "usage: bbc_micro_emu [options]\n\
         options:\n\
           --mos PATH                MOS (OS) ROM, 16 KiB\n\
           --lang BANK=PATH          language ROM (BANK 0..15)\n\
           --headless N              run for N cycles in headless mode, dump a PPM\n\
           --screenshot PATH         destination for the headless PPM (default screenshot.ppm)\n\
           --cycles N                cycles to run before opening window (warm-up)\n\
           --disk PATH               load .ssd disk image (8271 FDC; needs DFS ROM to be useful)\n\
           --type STRING             after the warm-up boot, type STRING into MOS\n\
                                     (newlines = CR; goes through OSRDCH passthrough)\n\
           --audio-out PATH          dump 0.5 s of SN76489 audio at the end of\n\
                                     a headless run as 22.05 kHz mono WAV\n\
           -h, --help                this help"
    );
}

#[derive(Default)]
struct Args {
    mos: Option<PathBuf>,
    banks: Vec<(u8, PathBuf)>,
    headless: Option<u64>,
    screenshot: Option<PathBuf>,
    warmup_cycles: u64,
    disk: Option<PathBuf>,
    type_str: Option<String>,
    audio_out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut iter = std::env::args().skip(1);
    let mut args = Args::default();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--mos" => args.mos = Some(PathBuf::from(iter.next().ok_or("--mos needs a path")?)),
            "--lang" => {
                let val = iter.next().ok_or("--lang needs <bank>=<path>")?;
                let (bank, path) = val.split_once('=').ok_or("--lang needs <bank>=<path>")?;
                let bank: u8 = bank.parse().map_err(|e| format!("bank: {e}"))?;
                args.banks.push((bank, PathBuf::from(path)));
            }
            "--headless" => {
                let val = iter.next().ok_or("--headless needs a cycle count")?;
                args.headless = Some(val.parse().map_err(|e| format!("cycles: {e}"))?);
            }
            "--screenshot" => {
                args.screenshot = Some(PathBuf::from(
                    iter.next().ok_or("--screenshot needs a path")?,
                ));
            }
            "--cycles" => {
                let val = iter.next().ok_or("--cycles needs a number")?;
                args.warmup_cycles = val.parse().map_err(|e| format!("cycles: {e}"))?;
            }
            "--disk" => {
                args.disk = Some(PathBuf::from(iter.next().ok_or("--disk needs a path")?));
            }
            "--type" => {
                args.type_str = Some(iter.next().ok_or("--type needs a string")?);
            }
            "--audio-out" => {
                args.audio_out = Some(PathBuf::from(
                    iter.next().ok_or("--audio-out needs a path")?,
                ));
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn build_machine(args: &Args) -> Result<Machine, Box<dyn std::error::Error>> {
    let mut mem = MemoryConfig {
        mos_rom_path: args.mos.clone(),
        ..MemoryConfig::default()
    };
    let mut initial_bank: u8 = 0;
    for (bank, path) in &args.banks {
        if (*bank as usize) >= mem.rom_banks.len() {
            return Err(format!("bank {bank} out of range").into());
        }
        mem.rom_banks[*bank as usize] = Some(path.clone());
        if *bank > initial_bank {
            initial_bank = *bank;
        }
    }
    mem.initial_bank = initial_bank;
    let mut machine = Machine::new(MachineConfig { memory: mem })?;
    if let Some(disk_path) = args.disk.as_ref() {
        let bytes = std::fs::read(disk_path)
            .map_err(|e| format!("cannot read disk image {}: {e}", disk_path.display()))?;
        machine.bus.hardware.fdc.load_ssd(bytes);
    }
    Ok(machine)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args().inspect_err(|_| print_usage())?;

    if let Some(cycles) = args.headless {
        let mut machine = build_machine(&args)?;
        let warmup = if args.warmup_cycles == 0 {
            5_000_000
        } else {
            args.warmup_cycles
        };
        machine.run_for_cycles(warmup, u64::MAX)?;
        if let Some(s) = &args.type_str {
            machine.type_string(s);
        }
        let report = machine.run_for_cycles(cycles, u64::MAX)?;
        let mut fb = Framebuffer::new();
        machine.render_into(&mut fb);
        let path = args
            .screenshot
            .clone()
            .unwrap_or_else(|| PathBuf::from("screenshot.ppm"));
        fb.save_ppm(&path)?;
        println!(
            "headless run: {} instructions, {} cycles, PC=${:04X}",
            report.instructions, report.cycles_spent, machine.cpu.registers.pc
        );
        println!("SHEILA: {}", machine.bus.hardware.access_summary());
        println!("CRTC R0..R17:");
        for chunk in (0..18).collect::<Vec<_>>().chunks(6) {
            let row: Vec<String> = chunk
                .iter()
                .map(|i| format!("R{i:>2}=${:02X}", machine.bus.hardware.crtc.reg(*i)))
                .collect();
            println!("  {}", row.join("  "));
        }
        println!(
            "Video ULA: CR=${:02X}  screen_size_code={}  bpp={}  teletext={}",
            machine.bus.hardware.video_ula.control,
            machine.bus.hardware.video_ula.screen_size_code,
            machine.bus.hardware.video_ula.bits_per_pixel(),
            machine.bus.hardware.video_ula.teletext_mode(),
        );
        println!("screenshot saved to {}", path.display());
        if let Some(audio_path) = args.audio_out.as_ref() {
            machine
                .bus
                .hardware
                .sound
                .dump_wav(audio_path, 22_050, 0.5)?;
            let aud = machine.bus.hardware.sound.is_audible();
            println!(
                "audio dumped to {}  (audible at end of run: {})",
                audio_path.display(),
                aud
            );
        }
        return Ok(());
    }

    let machine = build_machine(&args)?;
    let title = args
        .mos
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "no MOS".into());
    DisplayApp::new(machine, title).run()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
