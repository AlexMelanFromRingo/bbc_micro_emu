//! Address-space dispatcher implementing `mos6502_emu::Bus`.

use mos6502_emu::{Bus, MemoryView};

use crate::hardware::Hardware;
use crate::memory::Memory;

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
