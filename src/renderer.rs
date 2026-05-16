//! Software renderer: converts BBC screen RAM + CRTC + Video ULA state into a
//! 640×512 RGBA framebuffer (PAL-ish aspect). Renders the currently selected
//! mode by examining the Video ULA control register.

use crate::crtc::Crtc6845;
use crate::video_ula::VideoUla;

pub const SCREEN_W: usize = 640;
pub const SCREEN_H: usize = 512;

pub struct Framebuffer {
    pub pixels: Vec<u32>, // 0x00RRGGBB
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framebuffer {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; SCREEN_W * SCREEN_H],
        }
    }

    #[inline]
    fn put(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
        if x < SCREEN_W && y < SCREEN_H {
            let p = (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
            self.pixels[y * SCREEN_W + x] = p;
        }
    }

    fn clear(&mut self, rgb: [u8; 3]) {
        let p = (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
        for px in self.pixels.iter_mut() {
            *px = p;
        }
    }

    pub fn save_ppm(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(path)?;
        writeln!(f, "P6 {SCREEN_W} {SCREEN_H} 255")?;
        let mut bytes = Vec::with_capacity(SCREEN_W * SCREEN_H * 3);
        for px in &self.pixels {
            bytes.push(((*px >> 16) & 0xFF) as u8);
            bytes.push(((*px >> 8) & 0xFF) as u8);
            bytes.push((*px & 0xFF) as u8);
        }
        f.write_all(&bytes)?;
        Ok(())
    }
}

pub struct Renderer {
    pub frame_count: u32,
    /// 8×8 font for MODE 7 / fallback text. Populated from MOS ROM ($C000..$C300)
    /// when a MOS image is loaded; otherwise all zeros (blank screen).
    pub font: [u8; 96 * 8],
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            frame_count: 0,
            font: [0; 96 * 8],
        }
    }

    /// Populate the renderer's font cache from a 16 KiB MOS ROM image. The OS
    /// character set sits at the very start of MOS ($C000), one ASCII glyph
    /// (8 bytes) per code point from $20 onwards.
    pub fn set_font_from_mos(&mut self, mos: &[u8]) {
        let n = self.font.len().min(mos.len());
        self.font[..n].copy_from_slice(&mos[..n]);
    }

    pub fn render(&mut self, fb: &mut Framebuffer, ram: &[u8], crtc: &Crtc6845, ula: &VideoUla) {
        self.frame_count = self.frame_count.wrapping_add(1);
        fb.clear([0, 0, 0]);

        if std::env::var("RENDER_DEBUG").is_ok() {
            eprintln!(
                "render: teletext={} cols={} rows={} scanlines={} CR=${:02X} start=${:04X}",
                ula.teletext_mode(),
                crtc.horizontal_displayed(),
                crtc.vertical_displayed(),
                crtc.scanlines_per_char_row(),
                ula.control,
                crtc.display_start_crtc_addr()
            );
        }

        if ula.teletext_mode() {
            self.render_teletext(fb, ram, crtc, ula);
        } else {
            self.render_bitmap(fb, ram, crtc, ula);
        }
    }

    fn render_teletext(&self, fb: &mut Framebuffer, ram: &[u8], crtc: &Crtc6845, _ula: &VideoUla) {
        // MODE 7: 40 chars × 25 rows, screen RAM at $7C00 (1 KiB).
        // Each character cell rendered at 16×20 → 640×500 display area.
        let cols = crtc.horizontal_displayed().min(40) as usize;
        let rows = crtc.vertical_displayed().min(25) as usize;
        let start = crtc.display_start_crtc_addr();
        // The teletext screen base wraps so MA7..MA12 == $7C..$7F maps to $7C00.
        // For us, just compute the address: BBC base = $7C00 always for MODE 7.
        let base = 0x7C00u16;
        // CRTC start address: low 10 bits give offset within 1K teletext page.
        let offset = start & 0x03FF;
        for row in 0..rows {
            // Per-row state: foreground colour, doubled-height, graphics
            let mut fg: [u8; 3] = [255, 255, 255]; // default white
            let mut graphics = false;
            let mut sep = false;
            let mut hold: u8 = 0x20;
            let bg: [u8; 3] = [0, 0, 0];
            for col in 0..cols {
                let addr = base
                    .wrapping_add(offset)
                    .wrapping_add((row * cols) as u16)
                    .wrapping_add(col as u16);
                let code = ram[(addr & 0x7FFF) as usize] & 0x7F;

                // Process teletext control codes (codes 0x00-0x1F) in-band.
                let mut display_code = code;
                let mut new_graphics = graphics;
                let mut new_fg = fg;
                let mut new_sep = sep;
                if code < 0x20 {
                    display_code = if graphics { hold } else { 0x20 };
                    match code {
                        0x01 => new_fg = [255, 0, 0],     // alpha red
                        0x02 => new_fg = [0, 255, 0],     // alpha green
                        0x03 => new_fg = [255, 255, 0],   // alpha yellow
                        0x04 => new_fg = [0, 0, 255],     // alpha blue
                        0x05 => new_fg = [255, 0, 255],   // alpha magenta
                        0x06 => new_fg = [0, 255, 255],   // alpha cyan
                        0x07 => new_fg = [255, 255, 255], // alpha white
                        0x11 => {
                            new_fg = [255, 0, 0];
                            new_graphics = true;
                        }
                        0x12 => {
                            new_fg = [0, 255, 0];
                            new_graphics = true;
                        }
                        0x13 => {
                            new_fg = [255, 255, 0];
                            new_graphics = true;
                        }
                        0x14 => {
                            new_fg = [0, 0, 255];
                            new_graphics = true;
                        }
                        0x15 => {
                            new_fg = [255, 0, 255];
                            new_graphics = true;
                        }
                        0x16 => {
                            new_fg = [0, 255, 255];
                            new_graphics = true;
                        }
                        0x17 => {
                            new_fg = [255, 255, 255];
                            new_graphics = true;
                        }
                        0x10 => new_graphics = false, // alpha black (but spec says reset)
                        0x19 => new_sep = false,      // contiguous graphics
                        0x1A => new_sep = true,       // separated graphics
                        _ => {}
                    }
                }

                // Draw the character at (col*16, row*20) in 16×20 cell.
                let x0 = col * 16;
                let y0 = row * 20;
                if display_code >= 0x20 {
                    if graphics && display_code != 0x20 {
                        draw_teletext_graphics_cell(fb, x0, y0, display_code, sep, fg, bg);
                    } else {
                        draw_teletext_alpha_cell(fb, &self.font, x0, y0, display_code, fg, bg);
                    }
                } else {
                    fill_cell(fb, x0, y0, 16, 20, bg);
                }

                // Cursor: if this cell is the cursor address and the cursor
                // is currently visible, XOR-invert the cell. CRTC's cursor
                // register is in 14-bit MA space; for MODE 7 we mask to 10 bits
                // and add the screen base.
                let cursor_addr = base.wrapping_add(crtc.cursor_crtc_addr() & 0x03FF);
                if addr == cursor_addr && crtc.cursor_visible(self.frame_count) {
                    for dy in 0..20 {
                        for dx in 0..16 {
                            let i = (y0 + dy) * SCREEN_W + (x0 + dx);
                            if i < fb.pixels.len() {
                                fb.pixels[i] ^= 0x00FF_FFFF;
                            }
                        }
                    }
                }

                if code < 0x20 && graphics && display_code >= 0x20 {
                    hold = display_code;
                }
                fg = new_fg;
                graphics = new_graphics;
                sep = new_sep;
            }
        }
    }

    fn render_bitmap(&self, fb: &mut Framebuffer, ram: &[u8], crtc: &Crtc6845, ula: &VideoUla) {
        let cols = crtc.horizontal_displayed() as usize;
        let rows = crtc.vertical_displayed() as usize;
        let scanlines = crtc.scanlines_per_char_row() as usize;
        let start = crtc.display_start_crtc_addr();
        let screen_off = ula.screen_size_offset();
        let ula_mode = (ula.control >> 2) & 3;
        let high_clock = ula.high_clock();
        // Per b-em's render loop: HIFREQ (2 MHz CRTC) → 8 c-positions per
        // byte; LOFREQ → 16 c-positions per byte. Both target a 640-wide
        // display: 80 chars × 8 px = 640, 40 chars × 16 px = 640.
        let positions_per_byte: usize = if high_clock { 8 } else { 16 };
        let scale_y = (16 / scanlines.max(1)).max(1);

        for row in 0..rows {
            for col in 0..cols {
                for line in 0..scanlines {
                    let ma = start
                        .wrapping_add((row * cols) as u16)
                        .wrapping_add(col as u16);
                    let phys = Crtc6845::bbc_screen_addr(ma, line as u16, screen_off);
                    let byte = ram[phys as usize];
                    for c in 0..positions_per_byte {
                        // c is the horizontal pixel position within the byte's
                        // display area (0..7 in HIFREQ, 0..15 in LOFREQ).
                        // Per b-em `table4bpp[ula_mode][byte][c]`:
                        //
                        //   inner_c = c                                  (mode 3)
                        //   inner_c = c >> 1                             (mode 2)
                        //   inner_c = c >> 2                             (mode 1)
                        //   inner_c = c >> 3                             (mode 0)
                        //
                        // Within `inner_c`, the byte is left-shifted by that
                        // amount with 1-fill, then bits 1/3/5/7 form the 4-bit
                        // logical-colour index.
                        let inner_c = match ula_mode {
                            3 => c,
                            2 => c >> 1,
                            1 => c >> 2,
                            _ => c >> 3,
                        };
                        // HIFREQ uses positions 0..7 of the byte directly;
                        // LOFREQ stretches them — inner_c may exceed 7, but
                        // table[3] is identical for inner_c >= 8.
                        let inner_c = inner_c.min(15);
                        let shifted = if inner_c == 0 {
                            byte
                        } else {
                            ((byte as u16) << inner_c | ((1u16 << inner_c) - 1)) as u8
                        };
                        let logical = ((shifted >> 7) & 1) << 3
                            | ((shifted >> 5) & 1) << 2
                            | ((shifted >> 3) & 1) << 1
                            | ((shifted >> 1) & 1);
                        let color = ula.resolve_color(logical);
                        let x = col * positions_per_byte + c;
                        let y_base = (row * scanlines + line) * scale_y;
                        for dy in 0..scale_y {
                            fb.put(x, y_base + dy, color);
                        }
                    }
                }
            }
        }
    }
}

fn fill_cell(fb: &mut Framebuffer, x0: usize, y0: usize, w: usize, h: usize, rgb: [u8; 3]) {
    for dy in 0..h {
        for dx in 0..w {
            fb.put(x0 + dx, y0 + dy, rgb);
        }
    }
}

/// Render a teletext alphanumeric character at (x0, y0) as a 16×20 cell.
/// Uses the renderer's 8×8 font (loaded from MOS), scaled 2×; padded by 2 blank
/// rows top/bottom.
fn draw_teletext_alpha_cell(
    fb: &mut Framebuffer,
    font: &[u8; 96 * 8],
    x0: usize,
    y0: usize,
    ch: u8,
    fg: [u8; 3],
    bg: [u8; 3],
) {
    let glyph_base = if (0x20..0x80).contains(&ch) {
        (ch - 0x20) as usize * 8
    } else {
        0
    };
    fill_cell(fb, x0, y0, 16, 2, bg);
    for r in 0..8 {
        let row = font[glyph_base + r];
        for c in 0..8 {
            let lit = row & (0x80 >> c) != 0;
            let color = if lit { fg } else { bg };
            fb.put(x0 + c * 2, y0 + 2 + r * 2, color);
            fb.put(x0 + c * 2 + 1, y0 + 2 + r * 2, color);
            fb.put(x0 + c * 2, y0 + 2 + r * 2 + 1, color);
            fb.put(x0 + c * 2 + 1, y0 + 2 + r * 2 + 1, color);
        }
    }
    fill_cell(fb, x0, y0 + 18, 16, 2, bg);
}

/// Teletext "block graphics" cell: 2×3 grid of cells, where bits 0,1,2,3,4,6 of
/// the character byte (skipping bit 5) select which sub-cells are lit.
fn draw_teletext_graphics_cell(
    fb: &mut Framebuffer,
    x0: usize,
    y0: usize,
    ch: u8,
    sep: bool,
    fg: [u8; 3],
    bg: [u8; 3],
) {
    let bits = [
        ch & 0x01,
        ch & 0x02,
        ch & 0x04,
        ch & 0x08,
        ch & 0x10,
        ch & 0x40,
    ];
    let cell_w: usize = 8;
    let cell_h: [usize; 3] = [7, 6, 7]; // approximate 2:3 division of 20 vertical
    let mut y_off = 0usize;
    for r in 0..3 {
        let h = cell_h[r];
        for c in 0..2 {
            let bit = bits[r * 2 + c];
            let lit = bit != 0;
            let inset = if sep { 1 } else { 0 };
            for dy in 0..h {
                for dx in 0..cell_w {
                    let inside = lit
                        && dy >= inset
                        && dy < h.saturating_sub(inset)
                        && dx >= inset
                        && dx < cell_w.saturating_sub(inset);
                    let color = if inside { fg } else { bg };
                    fb.put(x0 + c * cell_w + dx, y0 + y_off + dy, color);
                }
            }
        }
        y_off += h;
    }
}
