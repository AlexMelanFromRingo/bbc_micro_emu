//! Aggregate of all SHEILA-mapped peripherals. For Phase 2 every device is a
//! logging stub that returns $FF and records accesses so we can see what MOS
//! actually pokes during boot.

use crate::acia6850::{Acia6850, SerialUla};
use crate::crtc::Crtc6845;
use crate::fdc8271::Fdc8271;
use crate::sheila::SheilaDevice;
use crate::sn76489::Sn76489;
use crate::system_via::SystemVia;
use crate::upd7002::UpD7002;
use crate::user_via::UserVia;
use crate::video_ula::VideoUla;

#[derive(Default, Clone)]
pub struct AccessLog {
    pub reads: u32,
    pub writes: u32,
    pub last_read_addr: u16,
    pub last_write_addr: u16,
    pub last_write_value: u8,
}

#[derive(Default)]
pub struct Hardware {
    pub log: [AccessLog; 11],
    pub crtc: Crtc6845,
    pub video_ula: VideoUla,
    pub system_via: SystemVia,
    pub user_via: UserVia,
    pub fdc: Fdc8271,
    pub sound: Sn76489,
    pub acia: Acia6850,
    pub serial_ula: SerialUla,
    pub adc: UpD7002,
}

impl Hardware {
    pub fn new() -> Self {
        Self::default()
    }

    fn log_idx(dev: SheilaDevice) -> usize {
        match dev {
            SheilaDevice::Crtc => 0,
            SheilaDevice::Acia => 1,
            SheilaDevice::SerialUla => 2,
            SheilaDevice::VideoUla => 3,
            SheilaDevice::RomSelect => 4,
            SheilaDevice::SystemVia => 5,
            SheilaDevice::UserVia => 6,
            SheilaDevice::Fdc => 7,
            SheilaDevice::Econet => 8,
            SheilaDevice::Adc => 9,
            SheilaDevice::Tube => 10,
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        let dev = SheilaDevice::from_addr(addr);
        let entry = &mut self.log[Self::log_idx(dev)];
        entry.reads = entry.reads.saturating_add(1);
        entry.last_read_addr = addr;
        match dev {
            SheilaDevice::Crtc => self.crtc.read(addr),
            SheilaDevice::VideoUla => self.video_ula.read(addr),
            SheilaDevice::SystemVia => self.system_via.read(addr as u8),
            SheilaDevice::UserVia => self.user_via.read(addr as u8),
            SheilaDevice::Fdc => self.fdc.read(addr as u8),
            SheilaDevice::Acia => self.acia.read(addr as u8),
            SheilaDevice::SerialUla => 0xFF, // write-only on real hardware
            SheilaDevice::Adc => self.adc.read(addr as u8),
            // Tube co-processor not present — return 0 so MOS / DFS see all
            // FIFO/status bits clear and don't sit in a Tube-data-ready loop.
            SheilaDevice::Tube => 0x00,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) -> Option<RomSelectWrite> {
        let dev = SheilaDevice::from_addr(addr);
        let entry = &mut self.log[Self::log_idx(dev)];
        entry.writes = entry.writes.saturating_add(1);
        entry.last_write_addr = addr;
        entry.last_write_value = value;

        match dev {
            SheilaDevice::Crtc => self.crtc.write(addr, value),
            SheilaDevice::VideoUla => self.video_ula.write(addr, value),
            SheilaDevice::SystemVia => {
                let prev_we = self.system_via.ic32 & 0x01 != 0; // /WE before
                self.system_via.write(addr as u8, value);
                // After a port B write the IC32 latch may have updated; sync
                // its mode-relevant bits over to the Video ULA.
                self.video_ula.screen_size_code = self.system_via.screen_size_code();
                // SN76489 latches on rising edge of /WE (bit 0 of IC32 going
                // from 0 to 1 — i.e. the chip was being written and the strobe
                // was just released).
                let new_we = self.system_via.ic32 & 0x01 != 0;
                if !prev_we && new_we {
                    self.sound.write(self.system_via.sound_latch);
                }
            }
            SheilaDevice::UserVia => self.user_via.write(addr as u8, value),
            SheilaDevice::Fdc => self.fdc.write(addr as u8, value),
            SheilaDevice::Acia => self.acia.write(addr as u8, value),
            SheilaDevice::SerialUla => self.serial_ula.write(value),
            SheilaDevice::Adc => self.adc.write(addr as u8, value),
            SheilaDevice::RomSelect => return Some(RomSelectWrite { bank: value & 0x0F }),
            _ => {}
        }
        None
    }

    pub fn poll_irq(&self) -> bool {
        self.system_via.poll_irq() || self.user_via.poll_irq() || self.acia.poll_irq()
    }

    /// Sample the FDC's NMI output once. Returns true if a rising edge has
    /// occurred since the last call; the edge is consumed, so subsequent
    /// calls return false until the next FDC interrupt.
    pub fn poll_nmi_edge(&mut self) -> bool {
        self.fdc.poll_nmi_edge()
    }

    pub fn access_summary(&self) -> String {
        let labels = [
            "CRTC", "ACIA", "SerULA", "VidULA", "ROMSel", "SysVIA", "UsrVIA", "FDC", "Econet",
            "ADC", "Tube",
        ];
        let mut parts = Vec::new();
        for (label, log) in labels.iter().zip(self.log.iter()) {
            if log.reads != 0 || log.writes != 0 {
                parts.push(format!("{label} r={} w={}", log.reads, log.writes));
            }
        }
        if parts.is_empty() {
            "(no SHEILA accesses)".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Returned from `Hardware::write` when the CPU wrote to the paged-ROM latch
/// so the bus can swap the sideways ROM bank.
pub struct RomSelectWrite {
    pub bank: u8,
}
