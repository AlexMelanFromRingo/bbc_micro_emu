//! Motorola 6845 CRT Controller.
//!
//! Only the register set and the frame/scanline counter logic is modelled;
//! actual pixel generation lives in `renderer.rs` (the CRTC tells it where the
//! current display frame starts and how big it is).
//!
//! Register summary (write to $FE00 = address latch, $FE01 = data):
//!
//! ```text
//!   R0  Horizontal total       (chars per scanline - 1)
//!   R1  Horizontal displayed   (visible chars per scanline)
//!   R2  Horizontal sync pos
//!   R3  Sync widths (V<<4 | H)
//!   R4  Vertical total         (char rows per frame - 1)
//!   R5  Vertical total adjust  (extra scanlines)
//!   R6  Vertical displayed     (visible char rows)
//!   R7  Vertical sync position
//!   R8  Interlace / skew
//!   R9  Maximum raster address (scanlines per char row - 1)
//!   R10 Cursor start raster + blink
//!   R11 Cursor end raster
//!   R12 Display start address high (6 bits)
//!   R13 Display start address low
//!   R14 Cursor position high (6 bits)
//!   R15 Cursor position low
//!   R16 Light pen high (read only)
//!   R17 Light pen low (read only)
//! ```
//!
//! The BBC Micro uses a quirky address translation between the CRTC's
//! 14-bit address output and the physical RAM address — see
//! [`Crtc6845::screen_base_address`] for the mapping.

pub const REG_COUNT: usize = 18;

pub struct Crtc6845 {
    addr_latch: u8,
    regs: [u8; REG_COUNT],
    /// Current scanline within the frame (0..vertical_total_scanlines).
    pub scanline_in_frame: u16,
    /// Cycle counter modulo cycles-per-scanline.
    cycle_in_scanline: u16,
    /// Set true on the scanline where VSync starts; cleared once consumed.
    vsync_edge: bool,
    /// Set true while the VSync output is active (CA1 of System VIA).
    pub vsync_active: bool,
    pending_half: u8,
    /// Snapshot of (R12,R13) per visible scanline (or 0xFFFF when unset). Lets
    /// the renderer query "what was the display start address at scanline Y?"
    /// — crucial for Elite-style mid-frame R12/R13 reprogramming.
    pub start_per_scanline: [u16; 312],
    /// Index into `start_per_scanline` for the current scanline.
    current_scanline_index: u16,
}

impl Default for Crtc6845 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crtc6845 {
    pub fn new() -> Self {
        Self {
            addr_latch: 0,
            regs: [0; REG_COUNT],
            scanline_in_frame: 0,
            cycle_in_scanline: 0,
            vsync_edge: false,
            vsync_active: false,
            pending_half: 0,
            start_per_scanline: [0; 312],
            current_scanline_index: 0,
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr & 0x01 {
            0 => 0xFF, // address register is write-only on the 6845
            _ => {
                // R14-R17 are readable; others return $00 on the SY6845E.
                match self.addr_latch & 0x1F {
                    14..=17 => self.regs[(self.addr_latch & 0x1F) as usize],
                    _ => 0x00,
                }
            }
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr & 0x01 {
            0 => self.addr_latch = value & 0x1F,
            _ => {
                let idx = (self.addr_latch & 0x1F) as usize;
                if idx < REG_COUNT {
                    self.regs[idx] = value;
                    // If R12 or R13 are updated mid-frame, propagate the new
                    // start to the rest of the current frame's scanlines so
                    // the renderer can pick it up. This lets Elite swap the
                    // 3D viewport for the HUD on the User VIA T2 interrupt.
                    if idx == 12 || idx == 13 {
                        let new_start = self.display_start_crtc_addr();
                        for slot in
                            &mut self.start_per_scanline[self.current_scanline_index as usize..]
                        {
                            *slot = new_start;
                        }
                    }
                }
            }
        }
    }

    pub fn reg(&self, idx: usize) -> u8 {
        self.regs[idx]
    }

    pub fn horizontal_displayed(&self) -> u16 {
        self.regs[1] as u16
    }

    pub fn vertical_displayed(&self) -> u16 {
        self.regs[6] as u16
    }

    pub fn scanlines_per_char_row(&self) -> u16 {
        self.regs[9] as u16 + 1
    }

    /// Cursor position in 14-bit CRTC address space.
    pub fn cursor_crtc_addr(&self) -> u16 {
        (((self.regs[14] & 0x3F) as u16) << 8) | self.regs[15] as u16
    }

    /// Cursor enabled (R10 bit 6:5 = 00 always on, 01 off, 10 blink slow, 11 blink fast).
    pub fn cursor_visible(&self, frame_count: u32) -> bool {
        let mode = (self.regs[10] >> 5) & 0b11;
        match mode {
            0 => true,                        // always on
            1 => false,                       // always off
            2 => (frame_count / 16) & 1 == 0, // ~1.5 Hz
            3 => (frame_count / 8) & 1 == 0,  // ~3 Hz
            _ => unreachable!(),
        }
    }

    /// Display start in 14-bit CRTC address space.
    pub fn display_start_crtc_addr(&self) -> u16 {
        (((self.regs[12] & 0x3F) as u16) << 8) | self.regs[13] as u16
    }

    /// BBC Micro screen-RAM physical address for CRTC character `ma` and raster
    /// `ra`. The CRTC's start register (R12:R13) is already pre-divided by 8,
    /// so physical = ma*8 + ra. When MA13 is set, the "screen wrap" hardware
    /// folds the address back into the visible screen region using the
    /// mode-dependent screen size (from IC32 bits 4-5 in System VIA, surfaced
    /// here as `screen_size_offset`).
    pub fn bbc_screen_addr(ma: u16, ra: u16, screen_size_offset: u16) -> u16 {
        let mut a = ma;
        if a & 0x2000 != 0 {
            // wrap: subtract the active screen size in bytes, divided by 8
            let wrap = screen_size_offset >> 3;
            a = a.wrapping_sub(wrap);
        }
        let phys = a.wrapping_mul(8).wrapping_add(ra & 0x07);
        phys & 0x7FFF
    }

    /// Advance the CRTC by `cycles` (2 MHz cycles in MODE 1/2/4/5/7, 1 MHz in
    /// MODE 0/3 — but for simplicity we just treat the clock as the 2 MHz CPU
    /// clock and rely on R0 to express the per-mode width). Returns events for
    /// the system to react to (VSync edges).
    /// Advance the CRTC by `cycles` of the CPU clock (2 MHz). The 6845 itself
    /// runs at 1 MHz so we consume two CPU cycles per character cell, carrying
    /// any leftover half-cycle into `pending_half`.
    pub fn tick(&mut self, cycles: u32) -> CrtcEvents {
        let mut events = CrtcEvents::default();
        let h_total = (self.regs[0] as u32 + 1).max(1); // characters per scanline
        let v_total_rows = self.regs[4] as u32 + 1;
        let v_adjust = self.regs[5] as u32;
        let raster_per_row = self.regs[9] as u32 + 1;
        let scanlines_per_frame = (v_total_rows * raster_per_row + v_adjust).max(1);

        // 1 character cell = 2 CPU cycles. Accumulate halves to avoid drift.
        let total_halves = cycles + self.pending_half as u32;
        let char_cells = total_halves / 2;
        self.pending_half = (total_halves % 2) as u8;
        let mut remaining = char_cells;
        while remaining > 0 {
            let chars_left_in_line = h_total - self.cycle_in_scanline as u32;
            let chars_now = remaining.min(chars_left_in_line);
            self.cycle_in_scanline += chars_now as u16;
            remaining -= chars_now;

            if self.cycle_in_scanline as u32 >= h_total {
                self.cycle_in_scanline = 0;
                self.scanline_in_frame = self.scanline_in_frame.wrapping_add(1);
                self.current_scanline_index = self
                    .scanline_in_frame
                    .min(self.start_per_scanline.len() as u16 - 1);
                if self.scanline_in_frame as u32 >= scanlines_per_frame {
                    self.scanline_in_frame = 0;
                    self.current_scanline_index = 0;
                    events.frame_done = true;
                    // Reset the per-scanline table to the current R12/R13 so
                    // the next frame starts coherent.
                    let start = self.display_start_crtc_addr();
                    for slot in &mut self.start_per_scanline {
                        *slot = start;
                    }
                }
                // VSync starts at scanline = R7 * (R9+1) and lasts for the V-sync width
                let vsync_start = self.regs[7] as u32 * raster_per_row;
                let vsync_width = ((self.regs[3] >> 4) & 0x0F) as u32;
                let vsync_width = if vsync_width == 0 { 16 } else { vsync_width };
                let vsync_end = vsync_start + vsync_width;
                let was_active = self.vsync_active;
                self.vsync_active = (self.scanline_in_frame as u32) >= vsync_start
                    && (self.scanline_in_frame as u32) < vsync_end;
                if self.vsync_active && !was_active {
                    self.vsync_edge = true;
                    events.vsync_edge = true;
                }
            }
        }
        events
    }

    /// Returns true if a VSync rising edge has been observed since the last
    /// `consume_vsync_edge()` call.
    pub fn consume_vsync_edge(&mut self) -> bool {
        std::mem::take(&mut self.vsync_edge)
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CrtcEvents {
    pub vsync_edge: bool,
    pub frame_done: bool,
}
