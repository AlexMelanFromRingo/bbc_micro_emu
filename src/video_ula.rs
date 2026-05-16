//! BBC Video ULA (custom Acorn part) at $FE20/$FE21.
//!
//! Two write-only registers (bit layout matches the original Acorn ULA, as
//! decoded by b-em and beebjit):
//!
//! ```text
//!   $FE20  Control register
//!     bit 7-5 Cursor display segments (cursor shape pattern)
//!     bit 4   1 = 6845 clocked at 2 MHz, 0 = 1 MHz
//!     bit 3-2 Pixels per CRTC clock (interleaved with chars-per-line via R0):
//!               11 = 8 pixels  (1bpp / MODE 0/3/4/6)
//!               10 = 4 pixels  (2bpp / MODE 1/5)
//!               01 = 2 pixels  (4bpp / MODE 2)
//!               00 = 1 pixel   (8bpp — only valid on NULA / VideoNuLA)
//!     bit 1   1 = teletext (MODE 7), 0 = bitmap
//!     bit 0   1 = flash colour swap
//!
//!   $FE21  Palette register (write-only)
//!     bit 7-4 Logical colour index (0-15)
//!     bit 3-0 Physical colour (bit 3 = flash, 2 = ~B, 1 = ~G, 0 = ~R; inverted)
//! ```
//!
//! Reads from $FE20/$FE21 return $FF (open bus on real hw).
//!
//! In addition the System VIA's IC32 (addressable latch) bits 4-5 select the
//! "screen size" — see [`screen_size_kib`].

pub struct VideoUla {
    pub control: u8,
    /// Logical → physical colour map. 16 entries; only the low 4 bits matter.
    pub palette: [u8; 16],
    /// Screen-size code (bits 4-5 of IC32 from System VIA). 0=20K (modes 0-2),
    /// 1=16K (mode 3), 2=10K (modes 4-5), 3=8K (modes 6-7).
    pub screen_size_code: u8,
}

impl Default for VideoUla {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoUla {
    pub fn new() -> Self {
        Self {
            control: 0,
            // Default palette: identity (logical N → physical N), with bit-inversion
            // matching how MOS programs it after RESET.
            palette: [0; 16],
            screen_size_code: 0,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr & 0x01 {
            0 => self.control = value,
            _ => {
                let logical = (value >> 4) & 0x0F;
                let physical = value & 0x0F;
                self.palette[logical as usize] = physical;
            }
        }
    }

    pub fn read(&self, _addr: u16) -> u8 {
        0xFF
    }

    /// Resolve a logical colour index (0..15, masked to the active mode) to a
    /// physical RGB triple (each component 0 or 255).
    ///
    /// Per b-em's `videoula_write`: the stored palette byte's low 4 bits are
    /// XOR'd with 7 (NOT 0xF — only the RGB lines are inverted, the flash bit
    /// is left alone). After XOR, bit 0 = R, bit 1 = G, bit 2 = B; bit 3 is
    /// the flash bit (not interpreted here).
    pub fn resolve_color(&self, logical: u8) -> [u8; 3] {
        let phys = self.palette[(logical & 0x0F) as usize] ^ 0x07;
        let r = if phys & 0x01 != 0 { 255 } else { 0 };
        let g = if phys & 0x02 != 0 { 255 } else { 0 };
        let b = if phys & 0x04 != 0 { 255 } else { 0 };
        [r, g, b]
    }

    /// Returns the offset to subtract from $8000 to find the start of screen RAM.
    /// Matches the standard BBC Micro screen sizes selected via IC32 bits 4-5.
    pub fn screen_size_offset(&self) -> u16 {
        match self.screen_size_code & 0x03 {
            0 => 0x5000, // 20 KiB (modes 0, 1, 2)
            1 => 0x4000, // 16 KiB (mode 3)
            2 => 0x2800, // 10 KiB (modes 4, 5)
            3 => 0x2000, // 8 KiB  (modes 6, 7)
            _ => unreachable!(),
        }
    }

    /// Convenience: bits per pixel from control bits 3:2.
    pub fn bits_per_pixel(&self) -> u8 {
        match (self.control >> 2) & 0x03 {
            0b00 => 8, // 1 pixel per byte — NULA only
            0b01 => 4, // 2 pixels per byte — MODE 2
            0b10 => 2, // 4 pixels per byte — MODE 1/5
            0b11 => 1, // 8 pixels per byte — MODE 0/3/4/6
            _ => unreachable!(),
        }
    }

    pub fn teletext_mode(&self) -> bool {
        self.control & 0x02 != 0
    }

    pub fn high_clock(&self) -> bool {
        self.control & 0x10 != 0
    }
}
