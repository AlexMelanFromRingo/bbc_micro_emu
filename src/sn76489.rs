//! Texas Instruments SN76489 (variant SN76489AN in the BBC Micro) — 4-channel
//! programmable sound generator.
//!
//! Three square-wave tone channels (0, 1, 2) and one noise channel (3). Each
//! channel has a 10-bit frequency divider and a 4-bit attenuator (volume).
//! Latched-byte command interface:
//!
//! ```text
//!   byte | reg type | meaning
//!   --------------------------------------------------------------
//!   1 ccc tttt    First byte. ccc = channel (3 bits, but bit 0
//!                 selects volume vs frequency, so 2 channel bits +
//!                 1 type bit). tttt = low 4 bits of new value.
//!   0 xx ffffff   Continuation. ffffff = high 6 bits of frequency.
//! ```
//!
//! In the BBC the SN76489 is fed via the slow data bus (port A of the System
//! VIA) and latched on the falling edge of IC32 bit 0 (`/SOUND_WE`).
//!
//! This module models the chip at byte granularity (the system VIA latches
//! one byte per write). It produces 16-bit signed mono samples on demand via
//! [`Sn76489::synthesize`].

/// 4 MHz / 16 = 250 kHz; this is the internal clock the chip uses to count
/// down each channel's frequency divider.
pub const CHIP_CLOCK_HZ: u32 = 4_000_000 / 16;

/// 16-step linear-in-dB attenuation table. Index 15 = silent, 0 = full volume.
/// Each step is roughly 2 dB on the real chip; we approximate with multipliers
/// of 0.79 each step (10 ** (-0.1) = 0.7943).
fn attenuation_table() -> [i16; 16] {
    const MAX: f32 = 8000.0; // peak amplitude per channel
    let mut t = [0i16; 16];
    let mut v = MAX;
    for slot in t.iter_mut().take(15) {
        *slot = v as i16;
        v *= 0.7943;
    }
    t[15] = 0;
    t
}

#[derive(Default)]
struct Channel {
    /// 10-bit divider value (latches on continuation write).
    period: u16,
    /// 4-bit attenuation (0 = loud, 15 = silent).
    attenuation: u8,
    /// Counter — counts down at chip_clock; flips output on underflow.
    counter: i32,
    /// Current square-wave output (0 = low, 1 = high).
    output: u8,
}

#[derive(Default)]
pub struct Sn76489 {
    channels: [Channel; 4],
    /// "Last touched" register: 0-7 where bit 0 selects vol(1)/freq(0) and
    /// bits 2:1 select the channel.
    latched_reg: u8,
    /// LFSR for the noise channel.
    noise_lfsr: u16,
    /// Noise control byte (bits 0-2 of the freq register).
    noise_ctrl: u8,
    /// Cached attenuation table.
    att_table: [i16; 16],
}

impl Sn76489 {
    pub fn new() -> Self {
        let mut s = Self {
            att_table: attenuation_table(),
            noise_lfsr: 0x8000,
            ..Default::default()
        };
        // On power-up all channels start at full attenuation (silent).
        for ch in &mut s.channels {
            ch.attenuation = 0x0F;
            ch.period = 1; // avoid division by zero on first sample
        }
        s
    }

    /// Write one byte to the chip via the slow-data bus.
    pub fn write(&mut self, value: u8) {
        if value & 0x80 != 0 {
            // "Latch" byte: %1 ccc tttt
            self.latched_reg = (value >> 4) & 0x07;
            let data = value & 0x0F;
            let ch = (self.latched_reg >> 1) as usize;
            if self.latched_reg & 1 == 1 {
                // Attenuation latch
                if ch < 4 {
                    self.channels[ch].attenuation = data;
                }
            } else if ch == 3 {
                // Noise control
                self.noise_ctrl = data;
                self.noise_lfsr = 0x8000;
            } else if ch < 4 {
                // Frequency low nibble — keep high bits.
                self.channels[ch].period = (self.channels[ch].period & 0x3F0) | data as u16;
            }
        } else {
            // "Data" byte: continuation of latched register, 6 bits of data.
            let data = (value & 0x3F) as u16;
            let ch = (self.latched_reg >> 1) as usize;
            if self.latched_reg & 1 == 1 {
                if ch < 4 {
                    self.channels[ch].attenuation = (data & 0x0F) as u8;
                }
            } else if ch == 3 {
                self.noise_ctrl = (data & 0x0F) as u8;
                self.noise_lfsr = 0x8000;
            } else if ch < 4 {
                self.channels[ch].period =
                    (self.channels[ch].period & 0x00F) | ((data & 0x3F) << 4);
            }
        }
    }

    /// Generate `n` 16-bit samples at the given output sample rate (Hz).
    pub fn synthesize(&mut self, sample_rate: u32, n: usize) -> Vec<i16> {
        let mut out = Vec::with_capacity(n);
        // Number of chip clocks per output sample.
        let clocks_per_sample = CHIP_CLOCK_HZ as i32 / sample_rate as i32;
        for _ in 0..n {
            // Step every channel forward by `clocks_per_sample` chip clocks.
            for ch_idx in 0..3 {
                let ch = &mut self.channels[ch_idx];
                ch.counter -= clocks_per_sample;
                while ch.counter <= 0 {
                    ch.output ^= 1;
                    ch.counter += ch.period.max(1) as i32;
                }
            }
            // Noise channel
            let noise_period: i32 = match self.noise_ctrl & 0x03 {
                0 => 0x10,
                1 => 0x20,
                2 => 0x40,
                _ => self.channels[2].period.max(1) as i32, // shared with tone 2
            };
            {
                let ch = &mut self.channels[3];
                ch.counter -= clocks_per_sample;
                while ch.counter <= 0 {
                    let new = if self.noise_ctrl & 0x04 != 0 {
                        // White noise feedback = bit 0 XOR bit 3
                        ((self.noise_lfsr >> 3) ^ self.noise_lfsr) & 1
                    } else {
                        self.noise_lfsr & 1 // periodic noise
                    };
                    self.noise_lfsr = (self.noise_lfsr >> 1) | (new << 15);
                    ch.output = (self.noise_lfsr & 1) as u8;
                    ch.counter += noise_period;
                }
            }

            let mut sample: i32 = 0;
            for ch in &self.channels {
                let amp = self.att_table[ch.attenuation as usize] as i32;
                if ch.output != 0 {
                    sample += amp;
                } else {
                    sample -= amp;
                }
            }
            sample = sample.clamp(i16::MIN as i32, i16::MAX as i32);
            out.push(sample as i16);
        }
        out
    }

    pub fn channel_period(&self, ch: usize) -> u16 {
        self.channels[ch].period
    }

    pub fn channel_attenuation(&self, ch: usize) -> u8 {
        self.channels[ch].attenuation
    }

    // ---- Snapshot accessors ----
    pub fn set_channel_period(&mut self, ch: usize, period: u16) {
        if ch < 4 {
            self.channels[ch].period = period;
        }
    }
    pub fn set_channel_attenuation(&mut self, ch: usize, att: u8) {
        if ch < 4 {
            self.channels[ch].attenuation = att;
        }
    }
    pub fn noise_ctrl_byte(&self) -> u8 {
        self.noise_ctrl
    }
    pub fn set_noise_ctrl_byte(&mut self, v: u8) {
        self.noise_ctrl = v;
    }

    /// True if any tone channel currently has non-silent attenuation
    /// AND a non-trivial period (i.e. the chip is actively producing
    /// audible output, not just sitting at power-on defaults).
    pub fn is_audible(&self) -> bool {
        self.channels
            .iter()
            .any(|c| c.attenuation < 0x0F && c.period > 1)
    }

    /// Write 16-bit signed mono PCM as a WAV file. Convenience for headless
    /// tests / `--audio-out` CLI mode. `sample_rate` is typically 44_100 or
    /// 22_050; `seconds` is the duration to synthesise from the chip's
    /// current state.
    pub fn dump_wav(
        &mut self,
        path: &std::path::Path,
        sample_rate: u32,
        seconds: f32,
    ) -> std::io::Result<()> {
        use std::io::Write;
        let n_samples = (sample_rate as f32 * seconds) as usize;
        let pcm = self.synthesize(sample_rate, n_samples);
        let data_size = (pcm.len() * 2) as u32;
        let mut f = std::fs::File::create(path)?;
        // RIFF / WAVE header — mono, 16-bit PCM.
        f.write_all(b"RIFF")?;
        f.write_all(&(36 + data_size).to_le_bytes())?;
        f.write_all(b"WAVEfmt ")?;
        f.write_all(&16u32.to_le_bytes())?; // fmt chunk size
        f.write_all(&1u16.to_le_bytes())?; // PCM
        f.write_all(&1u16.to_le_bytes())?; // mono
        f.write_all(&sample_rate.to_le_bytes())?;
        f.write_all(&(sample_rate * 2).to_le_bytes())?; // byte rate
        f.write_all(&2u16.to_le_bytes())?; // block align
        f.write_all(&16u16.to_le_bytes())?; // bits per sample
        f.write_all(b"data")?;
        f.write_all(&data_size.to_le_bytes())?;
        for s in pcm {
            f.write_all(&s.to_le_bytes())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latch_byte_sets_low_nibble_of_period() {
        let mut s = Sn76489::new();
        s.write(0x80 | 0x0A); // channel 0 frequency, low nibble = $A
        assert_eq!(s.channel_period(0) & 0x0F, 0x0A);
    }

    #[test]
    fn continuation_byte_sets_high_bits_of_period() {
        let mut s = Sn76489::new();
        s.write(0x80 | 0x0A); // latch channel 0 freq, low = $A
        s.write(0x3F); // continuation, high 6 bits = $3F
        let period = s.channel_period(0);
        assert_eq!(period & 0x0F, 0x0A);
        assert_eq!((period >> 4) & 0x3F, 0x3F);
    }

    #[test]
    fn volume_writes_set_attenuation() {
        let mut s = Sn76489::new();
        s.write(0x90 | 0x05); // channel 0 volume = 5
        assert_eq!(s.channel_attenuation(0), 5);
    }

    #[test]
    fn synthesize_produces_non_zero_for_loud_tone() {
        let mut s = Sn76489::new();
        s.write(0x80 | 0x04); // channel 0 period low = 4
        s.write(0x10); // period high = $10 (full period = $104 = 260)
        s.write(0x90); // channel 0 volume = 0 (loud)
        let samples = s.synthesize(44100, 1024);
        assert!(samples.iter().any(|&v| v != 0));
    }
}
