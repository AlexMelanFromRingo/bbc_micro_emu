//! Address-space dispatcher implementing `mos6502_emu::Bus`.

use std::cell::Cell;

use mos6502_emu::{Bus, MemoryView};

use crate::hardware::Hardware;
use crate::memory::Memory;

thread_local! {
    /// Most-recent CPU PC, set by `Machine::step_instruction` so the
    /// env-gated bus write-tracer can attribute writes to a code location.
    /// Not on the hot path — only read inside the BBC_WRITE_TRACE branch.
    pub static LAST_PC: Cell<u16> = const { Cell::new(0) };
    /// Ring buffer of recent PCs, populated only when BBC_BRK_TRACE is on.
    pub static PC_HISTORY: std::cell::RefCell<Vec<u16>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub struct BbcBus {
    pub memory: Memory,
    pub hardware: Hardware,
}

impl BbcBus {
    pub fn new(memory: Memory) -> Self {
        Self {
            memory,
            hardware: Hardware::new(),
        }
    }
}

impl MemoryView for BbcBus {
    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0xFBFF => self.memory.read(addr),
            // SHEILA is side-effect-heavy; peek returns a safe placeholder for the monitor.
            0xFC00..=0xFEFF => 0xFF,
            0xFF00..=0xFFFF => self.memory.read(addr),
        }
    }
}

impl Bus for BbcBus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0xFBFF => self.memory.read(addr),
            0xFC00..=0xFCFF => 0xFF, // FRED (external 1MHz bus, not implemented)
            0xFD00..=0xFDFF => 0xFF, // JIM  (external 1MHz bus, not implemented)
            0xFE00..=0xFEFF => self.hardware.read(addr),
            0xFF00..=0xFFFF => self.memory.read(addr),
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if let Ok(spec) = std::env::var("BBC_WRITE_TRACE") {
            // Spec format: hex address ranges separated by commas, e.g.
            // "10C9" or "1063-106F,10C9". Logs the bank for paged-ROM
            // context; PC isn't available here so use FDC_TRACE for that.
            for part in spec.split(',') {
                let (lo, hi) = if let Some((a, b)) = part.split_once('-') {
                    (
                        u16::from_str_radix(a.trim(), 16).unwrap_or(0),
                        u16::from_str_radix(b.trim(), 16).unwrap_or(0),
                    )
                } else {
                    let a = u16::from_str_radix(part.trim(), 16).unwrap_or(0);
                    (a, a)
                };
                if addr >= lo && addr <= hi {
                    let pc = LAST_PC.with(|c| c.get());
                    let bank = self.memory.selected_bank();
                    eprintln!("RAM W ${addr:04X}=${value:02X}  PC=${pc:04X} bank={bank}");
                }
            }
        }
        match addr {
            0x0000..=0xFBFF => self.memory.write(addr, value),
            0xFC00..=0xFCFF => {}
            0xFD00..=0xFDFF => {}
            0xFE00..=0xFEFF => {
                if let Some(rs) = self.hardware.write(addr, value) {
                    self.memory.select_bank(rs.bank);
                }
            }
            0xFF00..=0xFFFF => {} // top of MOS ROM is read-only
        }
    }
}
