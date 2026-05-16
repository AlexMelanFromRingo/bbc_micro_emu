//! BBC Micro System VIA wrapping the generic 6522.
//!
//! Special things compared to a bare 6522:
//!
//! * Port B (PB0-PB3) drives an addressable 3-to-8 latch ("IC32") via the
//!   "slow data bus". PB3 is the latch's input data; PB0-PB2 are the latch's
//!   address (which output bit to set/clear). The other PB pins read joystick
//!   button / speech state inputs.
//! * Port A is the slow-data-bus byte. It is shared by:
//!   - the keyboard (driving PA7 high when the addressed key is pressed),
//!   - the sound chip (PA0-PA7 is the SN76489 data byte),
//!   - the speech chip (PA0-PA7 is its data byte).
//!
//!   PA0-PA6 hold the row/column being scanned; PA7 is the keyboard sense bit.
//! * IC32 outputs select which slow-bus device is active:
//!   - bit 0: sound chip /WE       (active low → write sound register)
//!   - bit 1: speech chip /RDY     (write strobe)
//!   - bit 2: speech chip /RS      (read/write select)
//!   - bit 3: keyboard /AUTOSCAN   (0 = auto-scan disabled, hardware quiet;
//!     1 = auto-scan enabled, hardware sweeps cols)
//!   - bit 4: screen size bit 0
//!   - bit 5: screen size bit 1
//!   - bit 6: caps lock LED (active low)
//!   - bit 7: shift lock LED (active low)
//! * Keyboard matrix: 10 columns × 8 rows. PA0-PA3 select column (0..9), PA4-PA6
//!   select row (0..7). PA7 reads back as 1 if the selected key is pressed (manual
//!   scan) or if *any* key is pressed (auto-scan).
//! * CA1 input is wired to the 6845's /VSYNC; an edge there latches the
//!   vsync IRQ source. MOS scans the keyboard every CA1.
//! * CA2 input is wired to the keyboard's "auto-scan key down" output; it
//!   asserts when any key is pressed in auto-scan mode.

use crate::via6522::Via6522;

/// Full BBC Model B keyboard matrix entry. Each variant maps to a unique
/// (column, row) location in the 10×8 keyboard scanner. The mapping matches
/// the Acorn schematic (Issue 7 board) — verified against b-em's
/// `key_allegro2bbc` table in `src/keyboard.c`.
///
/// The byte encoding used by b-em is `(row << 4) | column`. We invert that
/// when computing [`BbcKey::matrix_pos`] so the returned tuple is
/// `(column, row)`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum BbcKey {
    // Row 0 — modifiers
    Shift,
    Ctrl,
    // Row 0 lock keys (electrical positions identical to Shift/Ctrl row but
    // separate matrix columns — see schematic)
    CapsLock,
    ShiftLock,
    // Function keys
    F0,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    // Top-row digits / symbols
    K1,
    K2,
    K3,
    K4,
    K5,
    K6,
    K7,
    K8,
    K9,
    K0,
    Minus,
    Equals, // `=` / `^` on BBC issue 4, separate key on issue 7
    Caret,  // `^` / `~`
    Backslash,
    Left,
    // Tab row (top alphabetic)
    Tab,
    KeyQ,
    KeyW,
    KeyE,
    KeyR,
    KeyT,
    KeyY,
    KeyU,
    KeyI,
    KeyO,
    KeyP,
    At,
    OpenBracket,
    Up,
    // Home row (alphabetic)
    KeyA,
    KeyS,
    KeyD,
    KeyF,
    KeyG,
    KeyH,
    KeyJ,
    KeyK,
    KeyL,
    Semicolon,
    Colon,
    CloseBracket,
    Return,
    // Bottom row
    KeyZ,
    KeyX,
    KeyC,
    KeyV,
    KeyB,
    KeyN,
    KeyM,
    Comma,
    Period,
    Slash,
    Delete,
    Copy, // BBC Copy key (acts as End)
    Down,
    Right,
    // Bottom
    Space,
    // Misc
    Escape,
    /// The BREAK key on the BBC is NOT in the matrix — it drives /RESET
    /// directly. Provided here so callers can request a soft reset.
    Break,
}

impl BbcKey {
    /// Returns the matrix position `(column, row)`. `column` is in 0..10,
    /// `row` is in 0..8. Returns (255, 255) for keys that are not part of the
    /// scannable matrix (e.g. [`BbcKey::Break`]).
    pub fn matrix_pos(self) -> (u8, u8) {
        use BbcKey::*;
        // Helper: byte = (row << 4) | col, matching b-em's encoding.
        const fn pos(byte: u8) -> (u8, u8) {
            (byte & 0x0F, byte >> 4)
        }
        match self {
            // Row 0
            Shift => pos(0x00),
            Ctrl => pos(0x01),
            // F-keys / digit row (row 1)
            F4 => pos(0x14),
            F0 => pos(0x20), // strictly in row 2 — kept here for ergonomic enum order
            F1 => pos(0x71),
            F2 => pos(0x72),
            F3 => pos(0x73),
            F5 => pos(0x74),
            F6 => pos(0x75),
            F7 => pos(0x16),
            F8 => pos(0x76),
            F9 => pos(0x77),
            // Digits
            K1 => pos(0x30),
            K2 => pos(0x31),
            K3 => pos(0x11),
            K4 => pos(0x12),
            K5 => pos(0x13),
            K6 => pos(0x34),
            K7 => pos(0x24),
            K8 => pos(0x15),
            K9 => pos(0x26),
            K0 => pos(0x27),
            Minus => pos(0x17),
            Equals => pos(0x18),
            Caret => pos(0x28),
            Backslash => pos(0x78),
            Left => pos(0x19),
            // Tab row
            Tab => pos(0x60),
            KeyQ => pos(0x10),
            KeyW => pos(0x21),
            KeyE => pos(0x22),
            KeyR => pos(0x33),
            KeyT => pos(0x23),
            KeyY => pos(0x44),
            KeyU => pos(0x35),
            KeyI => pos(0x25),
            KeyO => pos(0x36),
            KeyP => pos(0x37),
            At => pos(0x47),
            OpenBracket => pos(0x38),
            Up => pos(0x39),
            // Caps lock row
            CapsLock => pos(0x40),
            KeyA => pos(0x41),
            KeyS => pos(0x51),
            KeyD => pos(0x32),
            KeyF => pos(0x43),
            KeyG => pos(0x53),
            KeyH => pos(0x54),
            KeyJ => pos(0x45),
            KeyK => pos(0x46),
            KeyL => pos(0x56),
            Semicolon => pos(0x57),
            Colon => pos(0x48),
            CloseBracket => pos(0x58),
            Return => pos(0x49),
            // Shift lock row
            ShiftLock => pos(0x50),
            KeyZ => pos(0x61),
            KeyX => pos(0x42),
            KeyC => pos(0x52),
            KeyV => pos(0x63),
            KeyB => pos(0x64),
            KeyN => pos(0x55),
            KeyM => pos(0x65),
            Comma => pos(0x66),
            Period => pos(0x67),
            Slash => pos(0x68),
            Delete => pos(0x59),
            Copy => pos(0x69),
            Down => pos(0x29),
            Right => pos(0x79),
            // Space row
            Space => pos(0x62),
            // Misc
            Escape => pos(0x70),
            Break => (255, 255),
        }
    }
}

/// IC32 bit indices (output latch driven by Port B of System VIA).
pub mod ic32 {
    pub const SOUND_WE: u8 = 0;
    pub const SPEECH_RDY: u8 = 1;
    pub const SPEECH_RS: u8 = 2;
    pub const KEYBOARD_AUTOSCAN: u8 = 3;
    pub const SCREEN_SIZE_S0: u8 = 4;
    pub const SCREEN_SIZE_S1: u8 = 5;
    pub const CAPS_LOCK_LED: u8 = 6;
    pub const SHIFT_LOCK_LED: u8 = 7;
}

pub struct SystemVia {
    pub via: Via6522,
    /// 8-bit "IC32" addressable latch driven by Port B.
    pub ic32: u8,
    /// Keyboard matrix state: 10 columns, 8 rows each. `keys[col]` is a bitmask
    /// where bit `r` is 1 if the key at (col, r) is currently pressed.
    keys: [u8; 10],
    /// Capture of the previously-driven sound register (latched on /WE rising
    /// edge). Used by the optional sound chip emulation.
    pub sound_latch: u8,
    /// Edge tracker on /WE for sound writes.
    last_sound_we: bool,
}

impl Default for SystemVia {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemVia {
    pub fn new() -> Self {
        // IC32 (the 74LS259 addressable latch) powers up with undefined outputs
        // on real hardware. MOS explicitly initialises each bit during reset,
        // but it relies on the IC starting at 0 — matching b-em's
        // `sysvia_reset` which clears IC32 to $00. Starting at $FF means MOS's
        // "leave bit alone, it's already 0" assumptions are violated for
        // every bit MOS doesn't touch (notably the screen-size bits during
        // mode changes).
        Self {
            via: Via6522::new(),
            ic32: 0x00,
            keys: [0; 10],
            sound_latch: 0,
            last_sound_we: true,
        }
    }

    /// Press or release a key.
    pub fn set_key(&mut self, key: BbcKey, pressed: bool) {
        let (col, row) = key.matrix_pos();
        if col >= 10 || row >= 8 {
            return;
        }
        let mask = 1u8 << row;
        if pressed {
            self.keys[col as usize] |= mask;
        } else {
            self.keys[col as usize] &= !mask;
        }
    }

    /// True if any key is currently pressed (any column, any row).
    pub fn any_key_pressed(&self) -> bool {
        self.keys.iter().any(|c| *c != 0)
    }

    /// True if the key currently addressed by PA0-PA6 is pressed.
    fn selected_key_pressed(&self) -> bool {
        let pa = self.via.ora;
        let col = (pa & 0x0F) as usize;
        let row = ((pa >> 4) & 0x07) as usize;
        if col >= 10 {
            return false;
        }
        self.keys[col] & (1u8 << row) != 0
    }

    /// Compute the current PA7 input state per the BBC's keyboard logic. The
    /// polarity matches b-em's `sysvia_update_sdb`:
    ///
    /// * IC32 bit 3 = 1 (autoscan enabled): hardware sweeps the column
    ///   counter; PA7 stays "high" (matches CPU's written PA7 — typically
    ///   `1`). MOS detects activity via the CA2 line.
    /// * IC32 bit 3 = 0 (manual scan): when the addressed key is NOT pressed,
    ///   the keyboard pulls PA7 LOW. When the key IS pressed, PA7 stays high.
    fn pa7_state(&self) -> bool {
        let autoscan = self.ic32 & (1 << ic32::KEYBOARD_AUTOSCAN) != 0;
        if autoscan {
            // In autoscan, PA7 isn't pulled — it reflects whatever the CPU
            // last wrote (high bit of ORA). We treat it as "high" so MOS sees
            // its preloaded $80.
            true
        } else {
            // Manual scan: key down → PA7 high. Key not down → PA7 low.
            self.selected_key_pressed()
        }
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        // Refresh PA7 in IRA before serving the read so that port-A reads see
        // the live keyboard state.
        if self.pa7_state() {
            self.via.ira |= 0x80;
        } else {
            self.via.ira &= 0x7F;
        }
        self.via.read(reg)
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        self.via.write(reg, value);
        if reg & 0x0F == 0 {
            // Port B output drives the IC32 latch.
            //
            // Hardware: the latch is a 74LS259 8-bit addressable latch. PB0-PB2
            // select which of the 8 output bits to update; PB3 supplies the
            // new value for that bit. So a single port-B write toggles one bit
            // of IC32.
            let pb = value;
            let addr = (pb & 0x07) as usize;
            let data = pb & 0x08 != 0;
            let mask = 1u8 << addr;
            let prev_we = self.ic32 & (1 << ic32::SOUND_WE) != 0;
            if data {
                self.ic32 |= mask;
            } else {
                self.ic32 &= !mask;
            }
            // Detect /WE rising edge — that is the moment the SN76489 latches
            // the byte currently on the slow data bus.
            let new_we = self.ic32 & (1 << ic32::SOUND_WE) != 0;
            if prev_we && !new_we {
                // /WE pulled low → sound chip will latch on next /WE high
                self.last_sound_we = false;
            } else if !prev_we && new_we {
                self.sound_latch = self.via.ora;
                self.last_sound_we = true;
            }
        }
    }

    /// Update CA2 from the keyboard "key pressed" output. Per b-em's
    /// `key_update`:
    ///
    /// * Autoscan mode (IC32 bit 3 = 1): CA2 = 1 if ANY key in any column
    ///   (rows 1..7) is pressed. Modifier keys in row 0 (Shift, Ctrl) are
    ///   skipped — they have separate sense lines.
    /// * Manual scan (IC32 bit 3 = 0): CA2 = 1 if any key in the currently
    ///   addressed column (rows 1..7) is pressed.
    pub fn refresh_autoscan_ca2(&mut self) {
        let autoscan = self.ic32 & (1 << ic32::KEYBOARD_AUTOSCAN) != 0;
        let any = if autoscan {
            self.any_normal_key_pressed()
        } else {
            let col = (self.via.ora & 0x0F) as usize;
            if col < 10 {
                self.keys[col] & 0xFE != 0 // skip row 0 (modifiers)
            } else {
                false
            }
        };
        self.via.set_ca2(any);
    }

    /// True if any non-modifier key (any column, rows 1..7) is pressed.
    fn any_normal_key_pressed(&self) -> bool {
        self.keys.iter().any(|c| *c & 0xFE != 0)
    }

    /// Advance the VIA timers and refresh the keyboard CA2 line.
    pub fn tick(&mut self, cycles: u32) -> bool {
        let new_irq = self.via.tick(cycles);
        self.refresh_autoscan_ca2();
        new_irq || self.via.has_pending_irq()
    }

    /// Drive the CRTC's VSYNC output into CA1. The Motorola 6845 outputs
    /// VSYNC as active-HIGH and the BBC schematic ties it directly into the
    /// System VIA's CA1 input. MOS programs PCR for rising-edge detect, so
    /// passing `true` at the start of vertical retrace latches the vsync IRQ.
    pub fn pulse_vsync(&mut self, vsync_active: bool) {
        self.via.set_ca1(vsync_active);
    }

    pub fn poll_irq(&self) -> bool {
        self.via.has_pending_irq()
    }

    /// The BBC's screen-size selection comes from IC32 bits 4-5 (S0, S1). Per
    /// b-em sysvia.c: `scrsize = ((IC32 & 0x10) ? 2 : 0) | ((IC32 & 0x20) ? 1 : 0)`.
    /// So bit 4 → result bit 1 (high), bit 5 → result bit 0 (low).
    pub fn screen_size_code(&self) -> u8 {
        let bit4 = (self.ic32 >> ic32::SCREEN_SIZE_S0) & 1;
        let bit5 = (self.ic32 >> ic32::SCREEN_SIZE_S1) & 1;
        (bit4 << 1) | bit5
    }
}
