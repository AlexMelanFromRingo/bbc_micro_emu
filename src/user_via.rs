//! BBC Micro User VIA at $FE60-$FE7F.
//!
//! Same chip as the System VIA but its ports go to the user-port connector
//! (parallel I/O, printer, etc.) instead of the keyboard / IC32 latch. For
//! emulation purposes we just expose a plain 6522 — Elite famously uses User
//! VIA Timer 2 to switch CRTC parameters mid-frame for the split-screen 3D
//! window / 2D HUD effect.

use crate::via6522::Via6522;

#[derive(Default)]
pub struct UserVia {
    pub via: Via6522,
}

impl UserVia {
    pub fn new() -> Self {
        Self {
            via: Via6522::new(),
        }
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        self.via.read(reg)
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        self.via.write(reg, value);
    }

    pub fn tick(&mut self, cycles: u32) -> bool {
        self.via.tick(cycles);
        self.via.has_pending_irq()
    }

    pub fn poll_irq(&self) -> bool {
        self.via.has_pending_irq()
    }
}
