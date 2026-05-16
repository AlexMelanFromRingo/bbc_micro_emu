//! Top-level Machine: owns the CPU + bus and exposes a `run_for_cycles` loop.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use mos6502_emu::{Bus, Cpu, CpuError};

use crate::bus::BbcBus;
use crate::memory::{MOS_BASE, MOS_ROM_SIZE, Memory, MemoryConfig, RomLoadError};
use crate::renderer::Renderer;

/// Entry points of MOS vectored routines we intercept for the host-key
/// "type-ahead" path. Hitting OSRDCH ($FFE0) consumes one byte from the
/// machine's typed buffer; OSBYTE ($FFF4) handles a few common queries like
/// "is key X currently down?" so the negative-INKEY path works for in-game
/// keyboard scanning.
const OSRDCH_VEC: u16 = 0xFFE0;
const OSBYTE_VEC: u16 = 0xFFF4;

#[derive(Default)]
pub struct MachineConfig {
    pub memory: MemoryConfig,
}

#[derive(Debug)]
pub enum MachineError {
    Rom(RomLoadError),
    Cpu(CpuError),
}

impl Display for MachineError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rom(e) => write!(f, "ROM load error: {e}"),
            Self::Cpu(e) => write!(f, "CPU error: {e}"),
        }
    }
}

impl Error for MachineError {}

impl From<RomLoadError> for MachineError {
    fn from(value: RomLoadError) -> Self {
        Self::Rom(value)
    }
}

impl From<CpuError> for MachineError {
    fn from(value: CpuError) -> Self {
        Self::Cpu(value)
    }
}

pub struct Machine {
    pub cpu: Cpu,
    pub bus: BbcBus,
    pub renderer: Renderer,
    /// Cycles since CRTC was last ticked.
    cycles_since_crtc_tick: u64,
    /// FIFO of host characters typed by the user. Pulled one byte at a time
    /// when MOS executes the OSRDCH entry; see [`Machine::type_string`].
    typed_chars: VecDeque<u8>,
    /// If true, intercept MOS's OSRDCH / OSBYTE vectored entries to provide
    /// host-typed characters without having to scan the keyboard matrix. This
    /// lets us feed BASIC commands and (negative-INKEY) game polls reliably
    /// regardless of System VIA timing nuances.
    pub host_input_passthrough: bool,
}

impl Machine {
    pub fn new(config: MachineConfig) -> Result<Self, MachineError> {
        let memory = Memory::new(config.memory)?;
        let mut bus = BbcBus::new(memory);
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        let mut renderer = Renderer::new();
        // Snapshot the MOS font ($C000..$C300) for the renderer.
        let mut font_window = [0u8; 96 * 8];
        for (i, slot) in font_window.iter_mut().enumerate() {
            *slot = bus.memory.mos_byte(MOS_BASE.wrapping_add(i as u16));
        }
        renderer.set_font_from_mos(&font_window);
        let _ = MOS_ROM_SIZE; // keep import warning quiet
        Ok(Self {
            cpu,
            bus,
            renderer,
            cycles_since_crtc_tick: 0,
            typed_chars: VecDeque::new(),
            host_input_passthrough: true,
        })
    }

    /// Queue a string for the emulator to "type". Each byte is fed to MOS one
    /// at a time on subsequent OSRDCH calls. Newlines (\n) are translated to
    /// the BBC's CR (0x0D) which is what BASIC expects as a line terminator.
    pub fn type_string(&mut self, s: &str) {
        for b in s.bytes() {
            self.typed_chars
                .push_back(if b == b'\n' { 0x0D } else { b });
        }
    }

    pub fn type_byte(&mut self, b: u8) {
        self.typed_chars.push_back(b);
    }

    /// True if there are still queued characters waiting to be consumed.
    pub fn has_typed_chars(&self) -> bool {
        !self.typed_chars.is_empty()
    }

    /// Returns the number of queued host characters still waiting to be
    /// consumed by OSRDCH passthrough.
    pub fn typed_chars_len(&self) -> usize {
        self.typed_chars.len()
    }

    /// Attempt to intercept the MOS entry vector currently being fetched. If
    /// the CPU is about to execute the OSRDCH or OSBYTE entry AND the
    /// passthrough is enabled, we satisfy the call directly and return true
    /// (the caller skips the real instruction step).
    fn try_intercept(&mut self) -> bool {
        if !self.host_input_passthrough {
            return false;
        }
        // First, top up the MOS keyboard buffer if we have anything queued.
        // The buffer lives in RAM ($0300-$03FF with pointers at $02D8/$02E1)
        // so MOS will pick up the typed bytes via its standard REMV path —
        // no need to intercept the entry points themselves.
        self.flush_typed_into_keyboard_buffer();
        let pc = self.cpu.registers.pc;
        match pc {
            OSRDCH_VEC => self.intercept_osrdch(),
            OSBYTE_VEC => self.intercept_osbyte(),
            _ => false,
        }
    }

    /// MOS 1.20 keyboard buffer layout (Buffer 0):
    /// - base in RAM: $0300
    /// - insert (write) pointer: $02E1 (low byte)
    /// - remove (extract) pointer: $02D8 (low byte)
    ///
    /// The pointers wrap modulo 256 bytes — the entire $0300..$03FF page is
    /// available, though under normal MOS use only $03E0..$03FF is touched.
    fn flush_typed_into_keyboard_buffer(&mut self) {
        if self.typed_chars.is_empty() {
            return;
        }
        let ram = self.bus.memory.ram_mut();
        let mut insert = ram[0x02E1];
        let remove = ram[0x02D8];
        while let Some(&ch) = self.typed_chars.front() {
            let next = insert.wrapping_add(1);
            if next == remove {
                // Buffer full — wait for MOS to drain.
                break;
            }
            let buf_addr = 0x0300usize + insert as usize;
            ram[buf_addr] = ch;
            insert = next;
            self.typed_chars.pop_front();
        }
        let _ = remove;
        ram[0x02E1] = insert;
        // Refresh "buffer empty" flag at $0256 — MOS clears this when a
        // character is available. (Specific MOS 1.20 variable for OSRDCH wait.)
        ram[0x0256] = 0;
    }

    fn intercept_osrdch(&mut self) -> bool {
        // Pull next char from the typed FIFO; if empty we leave the call to
        // run through MOS so timing remains realistic.
        let Some(ch) = self.typed_chars.pop_front() else {
            return false;
        };
        self.cpu.registers.a = ch;
        // OSRDCH spec: C clear on success.
        let p = self.cpu.registers.status & !0x01;
        self.cpu.registers.status = p | mos6502_emu::UNUSED;
        // Pop the JSR return address and resume there.
        self.rts();
        // Charge a nominal 5 cycles so timing stays moving.
        self.cpu.cycles = self.cpu.cycles.wrapping_add(5);
        true
    }

    fn intercept_osbyte(&mut self) -> bool {
        // OSBYTE handles a *lot* of OS calls keyed by A. We only intercept the
        // ones that interact with host input; the rest fall through to MOS.
        match self.cpu.registers.a {
            // OSBYTE 0x81 = negative INKEY (read keyboard). X = -keycode (when
            // Y=$FF), or X=low/Y=high timeout (when polling). For the
            // negative-INKEY form we look at the typed queue: if it's empty
            // we say "no key", else we say "yes" with key code = first char.
            0x81 => {
                if self.cpu.registers.y != 0xFF {
                    return false;
                }
                if let Some(&ch) = self.typed_chars.front() {
                    self.cpu.registers.x = ch;
                    self.cpu.registers.y = 0;
                    let p = self.cpu.registers.status & !0x01; // C clear → key pressed
                    self.cpu.registers.status = p | mos6502_emu::UNUSED;
                } else {
                    self.cpu.registers.y = 0xFF;
                    let p = self.cpu.registers.status | 0x01; // C set → not pressed
                    self.cpu.registers.status = p;
                }
                self.rts();
                self.cpu.cycles = self.cpu.cycles.wrapping_add(8);
                true
            }
            _ => false,
        }
    }

    fn rts(&mut self) {
        let lo = self.read_stack();
        let hi = self.read_stack();
        let target = ((hi as u16) << 8) | lo as u16;
        self.cpu.registers.pc = target.wrapping_add(1);
    }

    fn read_stack(&mut self) -> u8 {
        self.cpu.registers.sp = self.cpu.registers.sp.wrapping_add(1);
        let addr = 0x0100u16 | self.cpu.registers.sp as u16;
        self.bus.read(addr)
    }

    pub fn step_instruction(&mut self) -> Result<u64, MachineError> {
        // Publish the about-to-execute PC for env-gated bus tracers.
        crate::bus::LAST_PC.with(|c| c.set(self.cpu.registers.pc));
        // Env-gated PC hit-counter — set BBC_PC_HIT=$XXXX[,$YYYY] to log
        // every time the CPU lands on one of those addresses (paged-ROM
        // bank in context). Useful for "did we reach the cleanup
        // routine?" diagnostics without a full trace.
        if let Ok(spec) = std::env::var("BBC_PC_HIT") {
            let pc = self.cpu.registers.pc;
            for part in spec.split(',') {
                let t = part.trim().trim_start_matches('$');
                if let Ok(addr) = u16::from_str_radix(t, 16)
                    && pc == addr
                {
                    let bank = self.bus.memory.selected_bank();
                    eprintln!("PC hit ${pc:04X} bank={bank}");
                }
            }
        }
        // Env-gated BRK-source trace: logs every $00 opcode (BRK) the CPU
        // is about to execute, with the bank it's executing from.
        if std::env::var("BBC_BRK_TRACE").is_ok() {
            let pc = self.cpu.registers.pc;
            // Read directly through MemoryView::peek so we don't trigger
            // side effects on SHEILA.
            use mos6502_emu::MemoryView;
            let op = self.bus.peek(pc);
            if op == 0 {
                let bank = self.bus.memory.selected_bank();
                eprintln!("BRK at ${pc:04X} bank={bank}");
            }
        }
        self.cpu.set_irq_line(self.bus.hardware.poll_irq());
        if self.bus.hardware.poll_nmi_edge() {
            self.cpu.request_nmi();
        }
        let cycles_before = self.cpu.cycles;
        if !self.try_intercept() {
            self.cpu.step(&mut self.bus)?;
        }
        let dt = self.cpu.cycles - cycles_before;
        self.cycles_since_crtc_tick += dt;
        // Tick CRTC in chunks of ~16 cycles to keep things smooth without
        // dominating the per-instruction cost.
        if self.cycles_since_crtc_tick >= 16 {
            let dt = self.cycles_since_crtc_tick as u32;
            let events = self.bus.hardware.crtc.tick(dt);
            // System VIA CA1 is wired to /VSYNC; the line is active-low on real
            // hw. We just mirror the CRTC's vsync_active state — Via6522::set_ca1
            // handles edge detection per PCR config (MOS sets falling edge).
            self.bus
                .hardware
                .system_via
                .pulse_vsync(self.bus.hardware.crtc.vsync_active);
            // Tick System and User VIA timers (T1/T2). Elite drives T2 of the
            // User VIA to fire mid-frame and swap CRTC parameters for the
            // 3D/HUD split — without timer support, the lower HUD never
            // displays.
            self.bus.hardware.system_via.tick(dt);
            self.bus.hardware.user_via.tick(dt);
            self.bus.hardware.fdc.tick(dt);
            self.bus.hardware.adc.tick(dt);
            if self.bus.hardware.adc.poll_eoc_edge() {
                // End-Of-Conversion is wired to CB1 on the System VIA.
                self.bus.hardware.system_via.via.set_cb1(true);
                self.bus.hardware.system_via.via.set_cb1(false);
            }
            self.cycles_since_crtc_tick = 0;
            let _ = events;
        }
        Ok(dt)
    }

    /// Run until the CPU has spent at least `target_cycles` cycles or `max_instructions`
    /// have been executed, whichever comes first.
    pub fn run_for_cycles(
        &mut self,
        target_cycles: u64,
        max_instructions: u64,
    ) -> Result<RunReport, MachineError> {
        let start_cycles = self.cpu.cycles;
        let mut executed = 0u64;

        while self.cpu.cycles.wrapping_sub(start_cycles) < target_cycles
            && executed < max_instructions
        {
            self.step_instruction()?;
            executed += 1;
        }

        Ok(RunReport {
            cycles_spent: self.cpu.cycles - start_cycles,
            instructions: executed,
        })
    }

    /// Run for ~one PAL frame (40000 cycles at 2 MHz = 20 ms). Used by the
    /// windowed display loop.
    pub fn run_one_frame(&mut self) -> Result<RunReport, MachineError> {
        self.run_for_cycles(40_000, u64::MAX)
    }

    pub fn render_into(&mut self, fb: &mut crate::renderer::Framebuffer) {
        self.renderer.render(
            fb,
            self.bus.memory.ram(),
            &self.bus.hardware.crtc,
            &self.bus.hardware.video_ula,
        );
    }
}

pub struct RunReport {
    pub cycles_spent: u64,
    pub instructions: u64,
}
