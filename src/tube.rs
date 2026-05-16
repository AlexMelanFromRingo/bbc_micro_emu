//! Acorn Tube ULA — second-processor (parasite) interface.
//!
//! The Tube is a memory-mapped FIFO pair between the BBC's host 6502 and a
//! parasite processor (typically a second 6502 at 3 MHz with its own 64 KiB
//! RAM). MOS / OS clients use it to offload compute-heavy work: Acornsoft
//! Tube Elite ships a "second-processor" client that executes the entire
//! game on the parasite, leaving the host purely for I/O.
//!
//! Register map at SHEILA `$FEE0..$FEE7` (host view):
//!
//! ```text
//!   $FEE0  R1 status                 $FEE1  R1 data
//!   $FEE2  R2 status                 $FEE3  R2 data
//!   $FEE4  R3 status                 $FEE5  R3 data
//!   $FEE6  R4 status                 $FEE7  R4 data
//! ```
//!
//! Each status byte reports:
//!   * bit 7 — DAV: data available to read from this register's "this side"
//!     queue.
//!   * bit 6 — NFL: not full, i.e. the "other side" queue still has room.
//!
//! In the BBC schematic R1 is the OS<->client command FIFO (single byte each
//! way), R2 is the per-byte transfer channel, R3 is the bulk data channel
//! (with a separate latch on the parasite side that holds a 2-byte word for
//! 16-bit transfers), and R4 is the interrupt-driven escape / event channel.
//! The host control register at `$FEE0` writes also reset the FIFOs and
//! enable IRQs — see [`Tube::write_control`].
//!
//! This implementation provides the *register interface* end-to-end (FIFOs +
//! status bits + control writes) plus a [`Parasite`] handle holding an
//! independent CPU + 64 KiB RAM. The bus between them is parameterised so
//! the parent emulator can step the parasite at its native 3 MHz rate
//! (1.5× the host) and route IRQs into either CPU.
//!
//! What's intentionally NOT modelled yet: synchronous handshakes (Tube
//! transfers are 2-cycle handshakes on real silicon; we accept any
//! ordering), parasite NMI from the host, and the optional DMA channel on
//! later Tube variants. Enough is in place for clients to detect a Tube,
//! exchange bytes, and execute code on the parasite.

use std::collections::VecDeque;

use mos6502_emu::cpu::Cpu;

/// Parasite RAM size — 64 KiB on every Acorn second processor.
pub const PARASITE_RAM_SIZE: usize = 0x10000;

/// 8 host-visible registers at $FEE0..$FEE7.
#[derive(Default)]
pub struct Tube {
    /// FIFO 1 — host ⇆ parasite, single byte each direction.
    r1_h2p: VecDeque<u8>,
    r1_p2h: VecDeque<u8>,
    /// FIFO 2 — same as R1 but a separate channel (often used as the
    /// in-band "escape" / control byte channel).
    r2_h2p: VecDeque<u8>,
    r2_p2h: VecDeque<u8>,
    /// FIFO 3 — bulk transfer. Real hardware buffers 2 bytes on the
    /// parasite-to-host side for 16-bit reads; we just round-trip bytes.
    r3_h2p: VecDeque<u8>,
    r3_p2h: VecDeque<u8>,
    /// FIFO 4 — IRQ / event channel.
    r4_h2p: VecDeque<u8>,
    r4_p2h: VecDeque<u8>,
    /// Control byte last written to R1 status (bit 6 enables Tube IRQs into
    /// host, bit 7 resets the FIFOs when written).
    control: u8,
    /// True if either parasite or host should raise IRQ. The owning
    /// `Machine` polls and routes to the appropriate CPU.
    pub host_irq: bool,
    pub parasite_irq: bool,
}

const FIFO_DEPTH_R1_R4: usize = 1;
const FIFO_DEPTH_R3: usize = 24;
const FIFO_DEPTH_R2: usize = 1;

impl Tube {
    pub fn new() -> Self {
        Self::default()
    }

    /// Host-side read of one of the 8 Tube registers ($FEE0..$FEE7).
    pub fn host_read(&mut self, reg: u8) -> u8 {
        match reg & 0x07 {
            0 => self.status_r1(),
            1 => self.r1_p2h.pop_front().unwrap_or(0),
            2 => self.status_r2(),
            3 => self.r2_p2h.pop_front().unwrap_or(0),
            4 => self.status_r3(),
            5 => self.r3_p2h.pop_front().unwrap_or(0),
            6 => self.status_r4(),
            7 => self.r4_p2h.pop_front().unwrap_or(0),
            _ => 0,
        }
    }

    /// Host-side write to one of the 8 Tube registers.
    pub fn host_write(&mut self, reg: u8, value: u8) {
        match reg & 0x07 {
            0 => self.write_control(value),
            1 => {
                if self.r1_h2p.len() < FIFO_DEPTH_R1_R4 {
                    self.r1_h2p.push_back(value);
                }
            }
            2 => {} // R2 status is read-only
            3 => {
                if self.r2_h2p.len() < FIFO_DEPTH_R2 {
                    self.r2_h2p.push_back(value);
                }
            }
            4 => {} // R3 status is read-only
            5 => {
                if self.r3_h2p.len() < FIFO_DEPTH_R3 {
                    self.r3_h2p.push_back(value);
                }
            }
            6 => {} // R4 status is read-only
            7 => {
                if self.r4_h2p.len() < FIFO_DEPTH_R1_R4 {
                    self.r4_h2p.push_back(value);
                    if self.control & 0x01 != 0 {
                        self.parasite_irq = true;
                    }
                }
            }
            _ => {}
        }
        self.refresh_irq();
    }

    /// Parasite-side read of one of the 8 Tube registers ($FEF8..$FEFF on
    /// the parasite's own SHEILA-equivalent).
    pub fn parasite_read(&mut self, reg: u8) -> u8 {
        match reg & 0x07 {
            0 => self.status_r1_parasite(),
            1 => self.r1_h2p.pop_front().unwrap_or(0),
            2 => self.status_r2_parasite(),
            3 => self.r2_h2p.pop_front().unwrap_or(0),
            4 => self.status_r3_parasite(),
            5 => self.r3_h2p.pop_front().unwrap_or(0),
            6 => self.status_r4_parasite(),
            7 => self.r4_h2p.pop_front().unwrap_or(0),
            _ => 0,
        }
    }

    /// Parasite-side write.
    pub fn parasite_write(&mut self, reg: u8, value: u8) {
        match reg & 0x07 {
            1 => {
                if self.r1_p2h.len() < FIFO_DEPTH_R1_R4 {
                    self.r1_p2h.push_back(value);
                }
            }
            3 => {
                if self.r2_p2h.len() < FIFO_DEPTH_R2 {
                    self.r2_p2h.push_back(value);
                }
            }
            5 => {
                if self.r3_p2h.len() < FIFO_DEPTH_R3 {
                    self.r3_p2h.push_back(value);
                }
            }
            7 => {
                if self.r4_p2h.len() < FIFO_DEPTH_R1_R4 {
                    self.r4_p2h.push_back(value);
                    if self.control & 0x10 != 0 {
                        self.host_irq = true;
                    }
                }
            }
            _ => {} // status registers are read-only
        }
        self.refresh_irq();
    }

    fn write_control(&mut self, value: u8) {
        self.control = value;
        if value & 0x80 != 0 {
            // Reset all FIFOs (bit 7 = soft reset)
            self.r1_h2p.clear();
            self.r1_p2h.clear();
            self.r2_h2p.clear();
            self.r2_p2h.clear();
            self.r3_h2p.clear();
            self.r3_p2h.clear();
            self.r4_h2p.clear();
            self.r4_p2h.clear();
            self.host_irq = false;
            self.parasite_irq = false;
        }
        self.refresh_irq();
    }

    fn refresh_irq(&mut self) {
        // Host IRQ from R1 (bit 5 of control) and R4 (bit 4 of control).
        let h = (self.control & 0x20 != 0 && !self.r1_p2h.is_empty())
            || (self.control & 0x10 != 0 && !self.r4_p2h.is_empty());
        if h {
            self.host_irq = true;
        }
        let p = (self.control & 0x02 != 0 && !self.r1_h2p.is_empty())
            || (self.control & 0x01 != 0 && !self.r4_h2p.is_empty());
        if p {
            self.parasite_irq = true;
        }
    }

    fn status_r1(&self) -> u8 {
        let mut s = 0;
        if !self.r1_p2h.is_empty() {
            s |= 0x80;
        }
        if self.r1_h2p.len() < FIFO_DEPTH_R1_R4 {
            s |= 0x40;
        }
        s | (self.control & 0x3F)
    }
    fn status_r1_parasite(&self) -> u8 {
        let mut s = 0;
        if !self.r1_h2p.is_empty() {
            s |= 0x80;
        }
        if self.r1_p2h.len() < FIFO_DEPTH_R1_R4 {
            s |= 0x40;
        }
        s
    }
    fn status_r2(&self) -> u8 {
        Self::two_fifo_status(self.r2_p2h.len(), self.r2_h2p.len(), FIFO_DEPTH_R2)
    }
    fn status_r2_parasite(&self) -> u8 {
        Self::two_fifo_status(self.r2_h2p.len(), self.r2_p2h.len(), FIFO_DEPTH_R2)
    }
    fn status_r3(&self) -> u8 {
        Self::two_fifo_status(self.r3_p2h.len(), self.r3_h2p.len(), FIFO_DEPTH_R3)
    }
    fn status_r3_parasite(&self) -> u8 {
        Self::two_fifo_status(self.r3_h2p.len(), self.r3_p2h.len(), FIFO_DEPTH_R3)
    }
    fn status_r4(&self) -> u8 {
        Self::two_fifo_status(self.r4_p2h.len(), self.r4_h2p.len(), FIFO_DEPTH_R1_R4)
    }
    fn status_r4_parasite(&self) -> u8 {
        Self::two_fifo_status(self.r4_h2p.len(), self.r4_p2h.len(), FIFO_DEPTH_R1_R4)
    }
    fn two_fifo_status(this_side_avail: usize, other_side_used: usize, depth: usize) -> u8 {
        let mut s = 0;
        if this_side_avail > 0 {
            s |= 0x80;
        }
        if other_side_used < depth {
            s |= 0x40;
        }
        s
    }

    pub fn poll_host_irq(&mut self) -> bool {
        let v = self.host_irq;
        self.host_irq = false;
        v
    }
    pub fn poll_parasite_irq(&mut self) -> bool {
        let v = self.parasite_irq;
        self.parasite_irq = false;
        v
    }
}

/// Parasite processor — a second 6502 with its own 64 KiB RAM, accessible
/// only via the Tube ULA. The owning `Machine` calls `step` from its main
/// loop; the parasite runs at the host clock rate × `ratio` (Acorn's 6502
/// second processor is 1.5× faster than the host).
pub struct Parasite {
    pub cpu: Cpu,
    pub ram: Box<[u8; PARASITE_RAM_SIZE]>,
    /// Optional reset ROM mapped at the top of RAM ($F000-$FFFF) when the
    /// parasite starts up. On real hardware the host streams the client
    /// code over R3 and the parasite jumps to it; we accept either path.
    pub boot_rom_top: Option<Box<[u8; 0x1000]>>,
}

impl Default for Parasite {
    fn default() -> Self {
        Self::new()
    }
}

impl Parasite {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            ram: Box::new([0xFFu8; PARASITE_RAM_SIZE]),
            boot_rom_top: None,
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        if let Some(rom) = self.boot_rom_top.as_ref()
            && addr >= 0xF000
        {
            return rom[(addr - 0xF000) as usize];
        }
        self.ram[addr as usize]
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        // ROM region is write-through to RAM so the parasite client can
        // patch its own boot vectors in place if it likes.
        self.ram[addr as usize] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tube_status_reports_no_data_and_space_available() {
        let mut t = Tube::new();
        // R1 status: DAV (bit 7) clear, NFL (bit 6) set.
        assert_eq!(t.host_read(0) & 0xC0, 0x40);
        assert_eq!(t.parasite_read(0) & 0xC0, 0x40);
    }

    #[test]
    fn host_to_parasite_round_trip_on_r1() {
        let mut t = Tube::new();
        t.host_write(1, 0x42);
        // Parasite sees DAV on R1.
        assert_eq!(t.parasite_read(0) & 0x80, 0x80);
        // Parasite reads the byte.
        assert_eq!(t.parasite_read(1), 0x42);
        // FIFO empty again.
        assert_eq!(t.parasite_read(0) & 0x80, 0x00);
    }

    #[test]
    fn parasite_to_host_round_trip_on_r3() {
        let mut t = Tube::new();
        for b in [0xAA, 0xBB, 0xCC] {
            t.parasite_write(5, b);
        }
        assert_eq!(t.host_read(4) & 0x80, 0x80, "DAV set after writes");
        let mut got = Vec::new();
        for _ in 0..3 {
            got.push(t.host_read(5));
        }
        assert_eq!(got, vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(t.host_read(4) & 0x80, 0x00, "DAV clear after drain");
    }

    #[test]
    fn control_reset_clears_all_fifos() {
        let mut t = Tube::new();
        t.host_write(1, 0x11);
        t.parasite_write(1, 0x22);
        t.host_write(0, 0x80); // soft reset bit
        assert_eq!(t.host_read(0) & 0x80, 0); // no DAV
        assert_eq!(t.parasite_read(0) & 0x80, 0);
    }

    #[test]
    fn host_irq_fires_when_r1_p2h_written_with_enable_set() {
        let mut t = Tube::new();
        t.host_write(0, 0x20); // enable host IRQ from R1
        t.parasite_write(1, 0x99);
        assert!(t.poll_host_irq());
        assert!(!t.poll_host_irq(), "edge consumed once");
    }

    #[test]
    fn parasite_64k_ram_round_trip() {
        let mut p = Parasite::new();
        p.write(0x4242, 0xA5);
        assert_eq!(p.read(0x4242), 0xA5);
        p.write(0x0000, 0x11);
        p.write(0xFFFF, 0x22);
        assert_eq!(p.read(0x0000), 0x11);
        assert_eq!(p.read(0xFFFF), 0x22);
    }
}
