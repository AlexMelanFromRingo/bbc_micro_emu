//! Generic Rockwell/Synertek 6522 Versatile Interface Adapter.
//!
//! Implements the subset the BBC Micro relies on:
//! * Port A and Port B with separate data direction registers
//! * Timer 1 (one-shot / free-running, with PB7 toggle optional)
//! * Timer 2 (one-shot)
//! * Interrupt Flag Register (IFR) + Interrupt Enable Register (IER)
//! * CA1 / CB1 inputs as level-triggered IRQ sources (used by the BBC for VSync
//!   and end-of-conversion).
//!
//! Things NOT yet implemented (not needed for booting MOS / running Elite):
//! * Shift register
//! * CA2 / CB2 output modes (PWM / pulse), only basic input edge detection
//! * Pulse-counting mode on T2
//!
//! Registers (offsets 0..15 from base):
//!
//! ```text
//!   0   IRB / ORB
//!   1   IRA / ORA
//!   2   DDRB
//!   3   DDRA
//!   4   T1C-L (read = counter low, clear T1 IFR; write = latch low)
//!   5   T1C-H (read = counter high; write = latch + reload, clear T1 IFR)
//!   6   T1L-L
//!   7   T1L-H (write = latch high, clear T1 IFR but do not reload)
//!   8   T2C-L (read = counter low, clear T2 IFR; write = latch low)
//!   9   T2C-H (read = counter high; write = latch+reload, clear T2 IFR)
//!  10   SR
//!  11   ACR
//!  12   PCR
//!  13   IFR (read = current flags; write = clear specified flags)
//!  14   IER (write bit 7 = enable, else disable; read = current mask)
//!  15   ORA (no handshake)
//! ```

pub const IFR_CA2: u8 = 0x01;
pub const IFR_CA1: u8 = 0x02;
pub const IFR_SR: u8 = 0x04;
pub const IFR_CB2: u8 = 0x08;
pub const IFR_CB1: u8 = 0x10;
pub const IFR_T2: u8 = 0x20;
pub const IFR_T1: u8 = 0x40;
pub const IFR_ANY: u8 = 0x80;

#[derive(Default)]
pub struct Via6522 {
    pub ora: u8,
    pub orb: u8,
    pub ira: u8, // mirrors of the input lines (peripherals write into these)
    pub irb: u8,
    pub ddra: u8,
    pub ddrb: u8,
    pub acr: u8,
    pub pcr: u8,
    pub ifr: u8,
    pub ier: u8,

    pub t1_counter: u16,
    pub t1_latch: u16,
    pub t1_running: bool,
    pub t2_counter: u16,
    pub t2_latch_lo: u8,
    pub t2_running: bool,

    /// Sampled state of CA1/CB1 control lines for edge detection.
    last_ca1: bool,
    last_cb1: bool,
    last_ca2: bool,
    last_cb2: bool,
    /// Half-cycle accumulator so that ticking from a 2 MHz CPU divides cleanly
    /// down to the 1 MHz Φ2 the chip is actually clocked from.
    pending_half: u8,
}

impl Via6522 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        let r = reg & 0x0F;
        match r {
            0 => {
                self.clear_ifr(IFR_CB1 | IFR_CB2);
                (self.orb & self.ddrb) | (self.irb & !self.ddrb)
            }
            1 => {
                self.clear_ifr(IFR_CA1 | IFR_CA2);
                (self.ora & self.ddra) | (self.ira & !self.ddra)
            }
            2 => self.ddrb,
            3 => self.ddra,
            4 => {
                self.clear_ifr(IFR_T1);
                self.t1_counter as u8
            }
            5 => (self.t1_counter >> 8) as u8,
            6 => self.t1_latch as u8,
            7 => (self.t1_latch >> 8) as u8,
            8 => {
                self.clear_ifr(IFR_T2);
                self.t2_counter as u8
            }
            9 => (self.t2_counter >> 8) as u8,
            10 => 0, // SR not implemented
            11 => self.acr,
            12 => self.pcr,
            13 => self.ifr_with_any(),
            14 => self.ier | 0x80,
            15 => (self.ora & self.ddra) | (self.ira & !self.ddra),
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        let r = reg & 0x0F;
        match r {
            0 => {
                self.orb = value;
                self.clear_ifr(IFR_CB1 | IFR_CB2);
            }
            1 => {
                self.ora = value;
                self.clear_ifr(IFR_CA1 | IFR_CA2);
            }
            2 => self.ddrb = value,
            3 => self.ddra = value,
            4 => self.t1_latch = (self.t1_latch & 0xFF00) | value as u16,
            5 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | ((value as u16) << 8);
                self.t1_counter = self.t1_latch;
                self.t1_running = true;
                self.clear_ifr(IFR_T1);
            }
            6 => self.t1_latch = (self.t1_latch & 0xFF00) | value as u16,
            7 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | ((value as u16) << 8);
                self.clear_ifr(IFR_T1);
            }
            8 => self.t2_latch_lo = value,
            9 => {
                self.t2_counter = ((value as u16) << 8) | self.t2_latch_lo as u16;
                self.t2_running = true;
                self.clear_ifr(IFR_T2);
            }
            10 => { /* SR ignored */ }
            11 => self.acr = value,
            12 => self.pcr = value,
            13 => {
                // Write to IFR: clear bits where written value has 1
                self.ifr &= !(value & 0x7F);
            }
            14 => {
                if value & 0x80 != 0 {
                    self.ier |= value & 0x7F;
                } else {
                    self.ier &= !(value & 0x7F);
                }
            }
            15 => self.ora = value,
            _ => {}
        }
    }

    /// Advance the timers by `cycles` CPU clocks. The VIA is clocked from the
    /// system Φ2 input which on the BBC Micro runs at 1 MHz — half the CPU
    /// clock — so we tick the internal counters every two CPU cycles.
    pub fn tick(&mut self, cycles: u32) -> bool {
        let before = self.has_pending_irq();
        // Accumulate half-cycles across calls to avoid drift.
        let total = cycles + self.pending_half as u32;
        let ticks = total / 2;
        self.pending_half = (total % 2) as u8;
        for _ in 0..ticks {
            if self.t1_running {
                if self.t1_counter == 0 {
                    // Overflow: latch T1 interrupt
                    self.set_ifr(IFR_T1);
                    if self.acr & 0x40 != 0 {
                        // free-run mode: reload from latch
                        self.t1_counter = self.t1_latch;
                    } else {
                        self.t1_running = false;
                    }
                } else {
                    self.t1_counter = self.t1_counter.wrapping_sub(1);
                }
            }
            // T2 only ticks on internal Φ2 clock when ACR bit 5 = 0
            // (interval-timer mode). With bit 5 = 1 the counter is fed by
            // PB6 transitions, which we don't drive — so it stays put.
            if self.t2_running && (self.acr & 0x20) == 0 {
                if self.t2_counter == 0 {
                    self.set_ifr(IFR_T2);
                    self.t2_running = false;
                } else {
                    self.t2_counter = self.t2_counter.wrapping_sub(1);
                }
            }
        }
        let after = self.has_pending_irq();
        !before && after
    }

    /// Notify CA1 input transition. PCR bit 0 selects edge: 0 = falling, 1 = rising.
    pub fn set_ca1(&mut self, level: bool) {
        let edge_rising = self.pcr & 0x01 != 0;
        let triggered = if edge_rising {
            !self.last_ca1 && level
        } else {
            self.last_ca1 && !level
        };
        if triggered {
            self.set_ifr(IFR_CA1);
        }
        self.last_ca1 = level;
    }

    pub fn set_cb1(&mut self, level: bool) {
        let edge_rising = self.pcr & 0x10 != 0;
        let triggered = if edge_rising {
            !self.last_cb1 && level
        } else {
            self.last_cb1 && !level
        };
        if triggered {
            self.set_ifr(IFR_CB1);
        }
        self.last_cb1 = level;
    }

    pub fn set_ca2(&mut self, level: bool) {
        // Only sense edges when configured as input (PCR bits 3:1 = 0xx).
        if self.pcr & 0x08 == 0 {
            let edge_rising = self.pcr & 0x04 != 0;
            let triggered = if edge_rising {
                !self.last_ca2 && level
            } else {
                self.last_ca2 && !level
            };
            if triggered {
                self.set_ifr(IFR_CA2);
            }
        }
        self.last_ca2 = level;
    }

    pub fn set_cb2(&mut self, level: bool) {
        if self.pcr & 0x80 == 0 {
            let edge_rising = self.pcr & 0x40 != 0;
            let triggered = if edge_rising {
                !self.last_cb2 && level
            } else {
                self.last_cb2 && !level
            };
            if triggered {
                self.set_ifr(IFR_CB2);
            }
        }
        self.last_cb2 = level;
    }

    pub fn has_pending_irq(&self) -> bool {
        self.ifr & self.ier & 0x7F != 0
    }

    fn ifr_with_any(&self) -> u8 {
        let mut v = self.ifr;
        if self.has_pending_irq() {
            v |= IFR_ANY;
        }
        v
    }

    fn set_ifr(&mut self, mask: u8) {
        self.ifr |= mask & 0x7F;
    }

    fn clear_ifr(&mut self, mask: u8) {
        self.ifr &= !(mask & 0x7F);
    }
}
