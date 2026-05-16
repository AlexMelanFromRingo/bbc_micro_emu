//! SHEILA $FE00-$FEFF: dispatch table for memory-mapped I/O devices.
//!
//! Each block is 32 bytes wide; on real hardware unused address bits are not
//! decoded, so e.g. the System VIA at $FE40-$FE4F is mirrored at $FE50-$FE5F.

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SheilaDevice {
    Crtc,      // $FE00-$FE07
    Acia,      // $FE08-$FE0F
    SerialUla, // $FE10-$FE1F
    VideoUla,  // $FE20-$FE2F
    RomSelect, // $FE30-$FE3F (write-only paged ROM latch)
    SystemVia, // $FE40-$FE5F
    UserVia,   // $FE60-$FE7F
    Fdc,       // $FE80-$FE9F
    Econet,    // $FEA0-$FEBF
    Adc,       // $FEC0-$FEDF
    Tube,      // $FEE0-$FEFF
}

impl SheilaDevice {
    pub const fn from_addr(addr: u16) -> Self {
        // Bits 4-7 of low byte select the device (mostly — sub-$FE20 area is finer).
        let low = addr as u8;
        match low {
            0x00..=0x07 => Self::Crtc,
            0x08..=0x0F => Self::Acia,
            0x10..=0x1F => Self::SerialUla,
            0x20..=0x2F => Self::VideoUla,
            0x30..=0x3F => Self::RomSelect,
            0x40..=0x5F => Self::SystemVia,
            0x60..=0x7F => Self::UserVia,
            0x80..=0x9F => Self::Fdc,
            0xA0..=0xBF => Self::Econet,
            0xC0..=0xDF => Self::Adc,
            0xE0..=0xFF => Self::Tube,
        }
    }
}
