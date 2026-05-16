//! NEC µPD7002 4-channel 10/12-bit analogue-to-digital converter.
//!
//! At $FE C0-$FE C3 on the BBC. The four input channels read analogue voltages
//! from the joystick connector (X/Y axes for two joysticks).
//!
//! Register map:
//!
//! ```text
//!   $FEC0  W = data latch (writes start a conversion of the addressed channel)
//!   $FEC0  R = status
//!   $FEC1  R = high byte of result
//!   $FEC2  R = low byte of result
//!   $FEC3  R = latch (reads write to data latch — used by 8-bit DDR access)
//! ```
//!
//! Status register:
//!
//! ```text
//!   bit 7  /BUSY   (1 = conversion in progress)
//!   bit 6  /CH1    (latched channel select bit)
//!   bit 5  /CH0    (latched channel select bit)
//!   bit 4  RES1    (latched resolution)
//!   bit 3  /MSB    (most-significant bit of result, mirrored for fast polling)
//!   bit 2  /LSB    (least-significant bit)
//!   bit 1-0       always 0 on real hardware
//! ```
//!
//! Conversion completes after ~4 ms (10-bit) or ~10 ms (12-bit) on real hw.
//! The ADC EOC pin is wired to System VIA CB1 — MOS scans the ADC via CB1
//! interrupts.

/// Analogue input (joystick axis). 16-bit logical signed value scaled into the
/// 10/12-bit output during conversion.
pub type AnalogValue = i16;

#[derive(Default)]
pub struct UpD7002 {
    /// Last-written channel + resolution control byte.
    pub control: u8,
    /// Result bytes (high, low). Always reflects the most recent conversion.
    pub result_hi: u8,
    pub result_lo: u8,
    /// /BUSY bit: 1 while a conversion is in progress.
    pub busy: bool,
    /// Cycles remaining until conversion completes (0 = idle).
    cycles_remaining: u32,
    /// Per-channel input values (signed -32768..32767 maps to 0..65535).
    inputs: [AnalogValue; 4],
    /// True when EOC has just fired and the CB1 line should be pulsed.
    eoc_edge: bool,
}

impl UpD7002 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the analogue input value for a given channel (0..=3).
    pub fn set_input(&mut self, channel: usize, value: AnalogValue) {
        if channel < 4 {
            self.inputs[channel] = value;
        }
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        match reg & 0x03 {
            0 | 3 => {
                self.control = value;
                // Start a new conversion on the addressed channel.
                self.busy = true;
                let resolution_12 = value & 0x10 != 0;
                self.cycles_remaining = if resolution_12 {
                    20_000 // ~10 ms at 2 MHz
                } else {
                    8_000 // ~4 ms at 2 MHz
                };
            }
            _ => {}
        }
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 0x03 {
            0 => {
                let mut s = 0u8;
                if self.busy {
                    s |= 0x80;
                }
                // Mirror channel select bits.
                s |= self.control & 0x30;
                s |= self.control & 0x04;
                // Top bits of result mirrored for fast polling.
                s |= (self.result_hi & 0xC0) >> 4;
                s
            }
            1 => self.result_hi,
            2 => self.result_lo,
            3 => self.control,
            _ => 0,
        }
    }

    pub fn tick(&mut self, cycles: u32) {
        if !self.busy {
            return;
        }
        if cycles >= self.cycles_remaining {
            self.busy = false;
            self.cycles_remaining = 0;
            self.finish_conversion();
        } else {
            self.cycles_remaining -= cycles;
        }
    }

    fn finish_conversion(&mut self) {
        let channel = (self.control & 0x03) as usize;
        let signed = self.inputs[channel];
        // Convert signed -32768..32767 to unsigned 0..65535 then truncate to
        // the configured resolution. Real ADC is straight binary.
        let unsigned = (signed as i32 + 32768) as u32 & 0xFFFF;
        let high = (unsigned >> 8) as u8;
        let low = (unsigned & 0xFF) as u8;
        self.result_hi = high;
        self.result_lo = low;
        self.eoc_edge = true;
    }

    /// Returns true if a conversion just completed (EOC). One-shot — clears
    /// itself once polled.
    pub fn poll_eoc_edge(&mut self) -> bool {
        std::mem::take(&mut self.eoc_edge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_conversion_sets_busy() {
        let mut adc = UpD7002::new();
        adc.write(0, 0x00); // channel 0, 10-bit
        assert!(adc.busy);
    }

    #[test]
    fn conversion_completes_after_cycle_budget() {
        let mut adc = UpD7002::new();
        adc.set_input(0, 12345);
        adc.write(0, 0x00);
        adc.tick(7_999);
        assert!(adc.busy);
        adc.tick(2);
        assert!(!adc.busy);
        // Result reflects the unsigned conversion of 12345.
        let expected = (12345i32 + 32768) as u32;
        let hi = (expected >> 8) as u8;
        let lo = (expected & 0xFF) as u8;
        assert_eq!(adc.read(1), hi);
        assert_eq!(adc.read(2), lo);
    }

    #[test]
    fn eoc_edge_fires_once_per_conversion() {
        let mut adc = UpD7002::new();
        adc.write(0, 0x00);
        adc.tick(10_000);
        assert!(adc.poll_eoc_edge());
        assert!(!adc.poll_eoc_edge());
    }
}
