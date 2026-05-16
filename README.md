# bbc_micro_emu

Acorn **BBC Micro Model B** (1981) emulator in safe Rust, built on top of the
sibling [`mos6502_emu`](https://github.com/AlexMelanFromRingo/mos6502_emu)
NMOS 6502 core. The long-term goal is running *Elite* (Acornsoft, 1984) from
its original `.ssd` disk image — every sub-system is implemented against the
real hardware (datasheets, service manuals, b-em as a cross-check) rather
than as a shortcut.

```
                COMMANDER JAMESON
─────────────────────────────────────────────
Present System              :Lave
Hyperspace System           :Lave
Condition                   :Docked
Fuel:7.0 Light Years
Cash:       100.0 Cr
Legal Status: Clean
Rating: Harmless

EQUIPMENT:
     Front Pulse Laser

         ┌────────────────────────────┐
         │ ╔══════════════════════╗   │
         │ ║ Cobra Mk III status  ║   │  ← Elite Docked screen,
         │ ╚══════════════════════╝   │     loaded from the original
         └────────────────────────────┘     Acornsoft ELITEBBC.SSD
```

## Status

| Component                        | Status |
| -------------------------------- | ------ |
| 6502 CPU core                    | ✅ All 151 documented + stable NMOS illegal opcodes; passes Klaus Dormann's `6502_functional_test` and `6502_interrupt_test` |
| 32 KiB RAM + 16 KiB MOS ROM      | ✅ |
| 16 paged sideways ROM banks      | ✅ MOS 1.20, BASIC II, DFS 0.98 all coexist |
| SHEILA memory-mapped dispatcher  | ✅ Per-device routing, access counters |
| Motorola 6845 CRTC               | ✅ All 18 registers, VSYNC/HSYNC timing, MA address generation |
| BBC Video ULA                    | ✅ Control reg, 16-entry palette (flash-bit aware), MODE 7 select, IC32 screen-size code |
| Software renderer                | ✅ MODE 7 (teletext, MOS font) + 1bpp/2bpp/4bpp bitmap modes; PPM dump |
| winit + softbuffer window        | ✅ Works in WSLg / X11 / Wayland |
| 6522 VIA (generic)               | ✅ Ports A/B, T1/T2 timers (free-run & pulse), shift register, IFR/IER, CA1/CB1 edges |
| System VIA                       | ✅ IC32 latch with correct power-on state, /VSYNC on CA1, keyboard matrix scan, ADC EOC on CB1 |
| User VIA                         | ✅ Available for split-screen T2 IRQs (needed by Elite) |
| Keyboard input                   | ✅ End-to-end: winit → System VIA matrix, plus a `--type` injector that drops bytes straight into MOS keyboard buffer 0 |
| 8271 FDC                         | ✅ Read Data / Write Data / Read Drive Status / Seek / Specify / Write Special Register; edge-triggered NMI + 128-cycle inter-byte pacing matches the timing DFS-090's NMI handler needs |
| `.ssd` disk image loading        | ✅ 200 KiB SSD images, undersized images zero-padded; DFS-090 fully drives the real Acornsoft Elite disc and renders its Docked screen |
| MOS service-call interception    | ✅ OSWRCH / OSRDCH / OSWORD / OSBYTE fast paths for headless testing |
| µPD7002 ADC                      | ⚙️ Skeleton with EOC pulse — joystick read works |
| 6850 ACIA                        | ⚙️ Stub (responds to register reads without wedging the CPU) |
| Sound (SN76489)                  | ⚙️ Stub (silent — keeps writes from looping) |
| Tube                             | — Not present |

`✅` = exercised by tests and verified visually. `⚙️` = enough behaviour to
keep MOS happy but not driving real audio/serial yet.

### Capabilities demonstrated

These all run from a single `cargo` invocation today:

```bash
# 1. Boot to BASIC and evaluate an expression
cargo run --release -- --mos roms/os120.rom --lang 15=roms/basic2.rom \
    --headless 8000000 --type "PRINT 1+1\n" --screenshot /tmp/boot.ppm
# → prints "2" on the BASIC prompt

# 2. Run a loop
cargo run --release -- --mos roms/os120.rom --lang 15=roms/basic2.rom \
    --headless 12000000 --type "FOR I=1 TO 5:PRINT I*I:NEXT\n"
# → 1 4 9 16 25

# 3. MODE 0 + bitmap text (80-column high-res)
cargo run --release -- --mos roms/os120.rom --lang 15=roms/basic2.rom \
    --headless 20000000 --type "MODE 0\nPRINT \"HELLO WORLD\"\n" \
    --screenshot /tmp/mode0.ppm

# 4. DFS-090 + Elite — *INFO !BOOT prints the file metadata
cargo run --release -- --mos roms/os120.rom --lang 15=roms/basic2.rom \
    --lang 14=roms/dfs090.rom --disk disks/Elite.ssd \
    --headless 30000000 --type "*INFO !BOOT\n" --screenshot /tmp/info.ppm
# → "$.!BOOT    L  001900 001E0F 00058A 157"

# 5. Boot the real Acornsoft Elite to the Docked / Commander Status screen
cargo run --release -- --mos roms/os120.rom --lang 15=roms/basic2.rom \
    --lang 14=roms/dfs090.rom --disk disks/Elite.ssd \
    --headless 320000000 --type "*RUN \$.!BOOT\n" --screenshot /tmp/elite.ppm
```

## Running

ROMs are © Acorn / successor rightholders and are **not** redistributed with
this crate. Fetch them from the public mdfs.net archive:

```bash
scripts/fetch_roms.sh         # → roms/os120.rom, roms/basic2.rom, roms/dfs098.rom
```

Then either open a window:

```bash
cargo run --release -- --mos roms/os120.rom --lang 15=roms/basic2.rom
```

…or run headless and dump a screenshot:

```bash
cargo run --release -- --mos roms/os120.rom --lang 15=roms/basic2.rom \
    --headless 8000000 --screenshot boot.ppm
```

CLI flags (see `--help` for the full list):

| Flag                  | Meaning                                                                |
| --------------------- | ---------------------------------------------------------------------- |
| `--mos PATH`          | 16 KiB MOS ROM (paged at `$C000`)                                      |
| `--lang BANK=PATH`    | sideways ROM in bank `0..15` (BASIC, DFS, …)                           |
| `--disk PATH`         | mount `.ssd` image in 8271 drive 0                                     |
| `--type STRING`       | after warm-up, push STRING into MOS keyboard buffer (`\n` → CR)        |
| `--headless N`        | run for `N` CPU cycles instead of opening a window                     |
| `--screenshot PATH`   | dump the framebuffer as PPM (headless only)                            |
| `--cycles N`          | extra warm-up cycles before opening the window                         |

## Tests

```bash
cargo fmt --all -- --check
cargo clippy --release --all-targets -- -D warnings
cargo test  --release                 # 43 unit / integration tests (no ROMs needed)
cargo test  --release -- --ignored    # adds ROM-dependent boot / render tests
```

Highlights:

- `tests/fdc_streaming.rs` — drives the 8271 the way a real 6502 NMI handler
  would and verifies a full 256-byte sector round-trips through Read Data /
  Write Data exactly.
- `tests/boot_to_basic.rs`, `tests/basic_eval.rs` — boot the real MOS + BASIC
  ROMs and assert on the screen RAM.
- `tests/mode4_draw.rs`, `tests/mode7_with_mos_font.rs` — render and count
  non-black pixels in the framebuffer.
- `tests/dfs_disk.rs` — mounts a synthetic DFS catalogue and exercises the
  DFS service ROM end-to-end.
- `tests/elite_disk.rs` — boots the real Acornsoft Elite `.ssd`, types
  `*RUN $.!BOOT`, and dumps screenshots through `Framebuffer::save_ppm`.
  Confirms `*INFO !BOOT` prints the catalogue line and that Elite reaches
  its Docked / Commander Status screen.

## Architecture

```
        +-------------+
        |    Cpu      |   ← mos6502_emu (NMOS 6502, all illegal opcodes)
        +------+------+
               | (Bus trait)
        +------v------+
        |   BbcBus    |   ← Sheila dispatcher
        +--+----+-----+
           |    |
   +-------v+   +v---------------+
   | Memory |   |   Hardware     |   ← per-device latched state
   +--------+   +-+--+--+--+--+--+
                  |  |  |  |  |
       +----------+  |  |  |  +-------+
       |             |  |  |          |
   +---v----+ +------v+ |  +------+ +-v----+
   |Crtc6845| |VideoULA| |SystemVia| |Fdc8271|
   +--------+ +--------+ +---------+ +-------+
                              ... UserVia, µPD7002, ACIA, SN76489
```

`Machine` ties them together and exposes `step_instruction`,
`run_for_cycles`, and `render_into(&mut Framebuffer)`. The `display` module
owns a winit event loop and pumps `run_one_frame` per redraw.

## Roadmap to Elite

1. ~~**DFS catalogue parse / file load.**~~ DONE — Acornsoft Elite boots
   to the Docked / Commander Status screen.
2. **In-game keyboard.** `Machine::type_string` lands chars in the MOS
   keyboard buffer for OSRDCH; Elite scans the VIA keyboard matrix
   directly during gameplay. `SystemVia::set_key` is wired but Elite's
   main loop doesn't react to taps yet — likely a VSYNC-IRQ pacing
   issue in our CRTC tick chunking.
3. **Scanline-accurate CRTC.** Elite reprograms `R12/R13` via a
   User-VIA T2 IRQ to split the 3D viewport from the dashboard. The
   CRTC tick already pulses VSYNC and our PPM rendering shows the
   bottom-half status bar correctly; the upper-half 3D viewport
   needs cycle-accurate mid-frame register sampling.
4. **Sound (SN76489).** Currently swallows writes silently. Elite
   plays laser, explosion, hyperspace and ECM tones.

## License

MIT — see [LICENSE](LICENSE). The CPU core lives in a sibling crate under
the same license.

Acorn MOS, BASIC and DFS ROMs are © Acorn Computers / Castle Technology /
RetroSoftware and are **not** included in this repository.
