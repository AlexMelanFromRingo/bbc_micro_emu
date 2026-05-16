//! Intel 8271 Floppy Disc Controller.
//!
//! BBC Model B (Acorn DFS 0.9 / 1.20) wires the 8271 to $FE80-$FE87 in SHEILA.
//! The chip drives /NMI on the 6502 whenever its INT flag is asserted; MOS's
//! NMI handler reads the appropriate register to acknowledge.
//!
//! Register map (offsets from $FE80):
//!
//! ```text
//!   0   Command (W) / Status (R)
//!   1   Parameter (W) / Result (R)
//!   2   Reset (W)
//!   4   Data (R/W)
//! ```
//!
//! Status register bits:
//!
//! ```text
//!   bit 7   COMMAND BUSY            (1 = command in progress)
//!   bit 6   COMMAND REG FULL
//!   bit 5   PARAMETER REG FULL
//!   bit 4   RESULT REG FULL
//!   bit 3   INT                     (1 = NMI asserted)
//!   bit 2   NON-DMA DATA REQUEST    (1 = byte in data register)
//!   bit 1   RESERVED                (0)
//!   bit 0   RESERVED                (0)
//! ```
//!
//! Implementation notes — this models the public-facing register interface and
//! the commands MOS / Acorn DFS issue during normal operation:
//!   * Specify             (0x35)  — set internal drive parameters
//!   * Read Drive Status   (0x2C)
//!   * Seek                (0x29)
//!   * Read ID             (0x1B)
//!   * Read Data           (0x12 / 0x13 / 0x16 / 0x17)
//!   * Write Data          (0x0A / 0x0B / 0x0E / 0x0F)
//!   * Verify              (0x1E / 0x1F)
//!   * Format Track        (0x23)
//!   * Read/Write Special Register (0x3D / 0x3A)
//!
//! The data transfer model is byte-at-a-time via NMI: the 8271 latches the
//! next sector byte into the Data register, raises INT + NDDR, and the NMI
//! handler reads the byte. Reading register 4 clears INT/NDDR; the FDC then
//! schedules the next byte after a cycle delay.
//!
//! Disk image format: this implementation accepts both SSD (single sided
//! double density, 80 × 10 × 256 = 200 KiB) and DSD (double sided, 400 KiB,
//! interleaved tracks). The drive being addressed determines which side of
//! a DSD image is used (drive 1 reads odd-numbered tracks of the DSD).

use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub const SECTOR_SIZE: usize = 256;
pub const SECTORS_PER_TRACK: usize = 10;
pub const TRACKS_PER_DISK: usize = 80;
pub const SSD_SIZE: usize = SECTOR_SIZE * SECTORS_PER_TRACK * TRACKS_PER_DISK;
pub const DSD_SIZE: usize = SSD_SIZE * 2;

#[derive(Debug)]
pub enum FdcError {
    BadImageSize { actual: usize },
    SeekOutOfRange { track: u8 },
}

impl Display for FdcError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadImageSize { actual } => write!(
                f,
                "disk image must be <= {SSD_SIZE} (SSD) or exactly {DSD_SIZE} (DSD) bytes; got {actual}"
            ),
            Self::SeekOutOfRange { track } => write!(f, "seek to invalid track {track}"),
        }
    }
}

impl Error for FdcError {}

/// Status register bits (the ones MOS / DFS poll for).
pub mod status {
    pub const BUSY: u8 = 0x80;
    pub const CMD_FULL: u8 = 0x40;
    pub const PARAM_FULL: u8 = 0x20;
    pub const RESULT_FULL: u8 = 0x10;
    pub const INTERRUPT: u8 = 0x08;
    pub const NDDR: u8 = 0x04;
}

/// Common 8271 result codes ("first byte" of the result returned after the
/// final NMI of each command).
pub mod result {
    pub const OK: u8 = 0x00;
    /// "Clock error" — used to signal a CRC failure on the data field.
    pub const DATA_CRC: u8 = 0x0E;
    pub const ID_CRC: u8 = 0x0C;
    pub const SECTOR_NOT_FOUND: u8 = 0x18;
    pub const DELETED_DATA: u8 = 0x20;
    pub const DRIVE_NOT_READY: u8 = 0x10;
    pub const WRITE_PROTECTED: u8 = 0x12;
}

/// State of one physical floppy drive.
#[derive(Default)]
struct Drive {
    /// Optional in-memory disk image. SSD or single side of DSD.
    image: Option<Vec<u8>>,
    /// True if this drive holds the second side of a DSD image.
    double_sided: bool,
    /// Currently-selected track on this drive.
    track: u8,
    /// Write-protect bit (DFS expects this to be 0 for normal operation).
    write_protect: bool,
}

impl Drive {
    fn loaded(&self) -> bool {
        self.image.is_some()
    }

    fn sector_offset(&self, side: u8, track: u8, sector: u8) -> Option<usize> {
        let track = track as usize;
        let sector = sector as usize;
        let side = side as usize;
        if track >= TRACKS_PER_DISK || sector >= SECTORS_PER_TRACK {
            return None;
        }
        let per_side = SSD_SIZE;
        let side_off = if self.double_sided {
            side * per_side
        } else {
            0
        };
        Some(side_off + (track * SECTORS_PER_TRACK + sector) * SECTOR_SIZE)
    }

    fn read_sector(&self, side: u8, track: u8, sector: u8) -> Option<&[u8]> {
        let off = self.sector_offset(side, track, sector)?;
        let image = self.image.as_ref()?;
        Some(&image[off..off + SECTOR_SIZE])
    }

    fn write_sector(&mut self, side: u8, track: u8, sector: u8, data: &[u8]) -> bool {
        if self.write_protect {
            return false;
        }
        let Some(off) = self.sector_offset(side, track, sector) else {
            return false;
        };
        let Some(image) = self.image.as_mut() else {
            return false;
        };
        let n = data.len().min(SECTOR_SIZE);
        image[off..off + n].copy_from_slice(&data[..n]);
        true
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum Phase {
    Idle,
    /// Awaiting `paramreq` parameter bytes.
    GatherParams {
        paramreq: u8,
    },
    /// Streaming a sector to the CPU one byte at a time via NMI.
    Reading {
        track: u8,
        sector: u8,
        side: u8,
        bytes_left: u32,
        offset: usize,
        sectors_left: u8,
    },
    /// Streaming a sector from the CPU one byte at a time via NMI.
    Writing {
        track: u8,
        sector: u8,
        side: u8,
        bytes_left: u32,
        offset: usize,
        sectors_left: u8,
        buffer: Vec<u8>,
    },
    /// Final delay after the last byte of a read completes — schedules the
    /// command-completion NMI (per b-em's `i8271_finishread` semantics).
    FinishingRead,
    /// Completing a command — result already in result register, INT asserted.
    Complete,
}

pub struct Fdc8271 {
    status: u8,
    result: u8,
    data: u8,
    command: u8,
    params: [u8; 5],
    paramnum: u8,
    paramreq: u8,
    /// Microseconds (at 2 MHz CPU) until next state transition. 0 = immediate.
    delay: u32,
    drives: [Drive; 2],
    cur_drive: usize,
    /// Selected side (for DSD images) — driven by external bit on drive port.
    side: u8,
    phase: Phase,
    /// Special / MMIO registers. The Acorn DFS-098 service ROM reaches well
    /// past the documented "internal" register set (0x00..0x1F) to talk to
    /// the MMIO ports — $23 is `mmioDriveOut`, $24 is `mmioClocks`, $25 is
    /// `mmioData` (see jsbeeb intel-fdc.js). Sizing the array to 64 lets
    /// any 6-bit register index be addressed without aliasing the latches
    /// for drive-select / step / load-head.
    special: [u8; 64],
    /// Latched NMI line state.
    nmi_pending: bool,
    /// Specify-command parameters (step rate, head settle, head load, etc.).
    step_rate: u8,
    head_settle: u8,
    head_load: u8,
    /// Cycle counter for the periodic index pulse. Real 5.25" drives spin at
    /// 300 rpm = 200 ms / revolution. At 2 MHz CPU clock that's 400 000
    /// cycles; the index pulse asserts for ~3 ms (6 000 cycles) per turn.
    rotation_cycle: u32,
    /// Countdown (in CPU cycles) before a freshly-selected drive's RDY/INDEX
    /// bits go high. Real drives need 0.5-1 s to spin up; we model this with
    /// a smaller window — long enough that DFS sees not-ready then ready
    /// across consecutive Read Drive Status polls.
    spin_up_remaining: u32,
}

impl Default for Fdc8271 {
    fn default() -> Self {
        Self::new()
    }
}

impl Fdc8271 {
    pub fn new() -> Self {
        Self {
            status: 0,
            result: 0,
            data: 0,
            command: 0,
            params: [0; 5],
            paramnum: 0,
            paramreq: 0,
            delay: 0,
            drives: Default::default(),
            cur_drive: 0,
            side: 0,
            phase: Phase::Idle,
            special: [0; 64],
            nmi_pending: false,
            step_rate: 0,
            head_settle: 0,
            head_load: 0,
            rotation_cycle: 0,
            spin_up_remaining: 0,
        }
    }

    /// Load a disk image (SSD = 200 KiB or DSD = 400 KiB) into the specified drive.
    pub fn load_image(&mut self, drive: usize, bytes: Vec<u8>) -> Result<(), FdcError> {
        if drive >= 2 {
            return Err(FdcError::SeekOutOfRange { track: drive as u8 });
        }
        match bytes.len() {
            DSD_SIZE => {
                self.drives[drive].image = Some(bytes);
                self.drives[drive].double_sided = true;
            }
            // Real DFS images can be short — only sectors that contain data
            // are stored. Pad up to a full 80-track SSD so out-of-range reads
            // return $00 rather than panicking; mark single-sided.
            n if n <= SSD_SIZE => {
                let mut padded = bytes;
                padded.resize(SSD_SIZE, 0);
                self.drives[drive].image = Some(padded);
                self.drives[drive].double_sided = false;
            }
            other => return Err(FdcError::BadImageSize { actual: other }),
        }
        Ok(())
    }

    /// Convenience for the CLI: load SSD bytes into drive 0.
    pub fn load_ssd(&mut self, bytes: Vec<u8>) {
        let _ = self.load_image(0, bytes);
    }

    pub fn has_disk(&self) -> bool {
        self.drives.iter().any(|d| d.loaded())
    }

    /// True if an NMI rising edge should be delivered to the CPU. Reading the
    /// signal also clears it (one NMI per edge).
    pub fn poll_nmi_edge(&mut self) -> bool {
        let edge = self.nmi_pending;
        self.nmi_pending = false;
        if edge && std::env::var("FDC_TRACE_NMI").is_ok() {
            eprintln!("NMI fire: status=${:02X}", self.status);
        }
        edge
    }

    /// True when the index hole is currently passing the read head. Real
    /// 5.25" drives spin at 300 rpm — 200 ms / revolution with the index
    /// pulse asserted for ~4 ms of that. To make DFS's "wait for the
    /// index pulse to come around" loops succeed reliably without
    /// implementing per-sector rotation timing, we widen the window to
    /// ~10 % of the revolution and stagger the off-time so back-to-back
    /// status polls observe the pulse going both low and high.
    fn index_active(&self) -> bool {
        self.rotation_cycle < 40_000
    }

    /// Advance internal state by `cycles` CPU clocks.
    pub fn tick(&mut self, cycles: u32) {
        self.rotation_cycle = (self.rotation_cycle + cycles) % 400_000;
        self.spin_up_remaining = self.spin_up_remaining.saturating_sub(cycles);
        if cycles == 0 {
            return;
        }
        if self.delay > cycles {
            self.delay -= cycles;
            return;
        }
        let overflow = cycles - self.delay;
        self.delay = 0;
        self.advance_phase(overflow);
    }

    fn advance_phase(&mut self, _slack: u32) {
        // Don't yank phases that need to persist across ticks (Idle and
        // GatherParams). For phases that consume data per tick we use
        // mem::replace so the inline value can be re-bound to a new state.
        if matches!(self.phase, Phase::Idle | Phase::GatherParams { .. }) {
            return;
        }
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle | Phase::GatherParams { .. } => {}
            Phase::Reading {
                track,
                sector,
                side,
                mut bytes_left,
                mut offset,
                mut sectors_left,
            } => {
                // Pace the data stream against the CPU. If the previous byte
                // is still in the data register (NDDR set), wait — the CPU
                // hasn't consumed it yet. This matches the real 8271 which
                // holds the byte until the CPU reads $FE84 (clearing NDDR).
                if self.status & status::NDDR != 0 {
                    // Restore phase and try again next tick.
                    self.phase = Phase::Reading {
                        track,
                        sector,
                        side,
                        bytes_left,
                        offset,
                        sectors_left,
                    };
                    self.delay = 16;
                    return;
                }
                // Latch next data byte into the data register and raise NMI.
                // The 8271 reports BUSY | INTERRUPT | NDDR while a byte is
                // pending in the data register (b-em's `i8271_data`).
                if let Some(sec) = self.drives[self.cur_drive].read_sector(side, track, sector) {
                    if offset < sec.len() {
                        self.data = sec[offset];
                    } else {
                        self.data = 0;
                    }
                } else {
                    self.complete_with(result::SECTOR_NOT_FOUND);
                    return;
                }
                offset += 1;
                bytes_left = bytes_left.saturating_sub(1);
                // Status during byte transfer: BUSY + NDDR + INTERRUPT (no
                // RESULT_FULL — that comes only with the final completion).
                // NMI is edge-triggered on the rising edge of INT; only
                // pulse it if INT was 0 before. Otherwise the CPU sees a
                // run of spurious NMIs and nests them onto the stack.
                let was_int = self.status & status::INTERRUPT != 0;
                self.status = status::BUSY | status::NDDR | status::INTERRUPT;
                if !was_int {
                    self.nmi_pending = true;
                }
                if bytes_left == 0 {
                    // Sector finished. Move to next sector or schedule the
                    // command-completion NMI after a short delay (matches
                    // b-em's `i8271_finishread` which sets fdc_time = 200).
                    if sectors_left > 1 {
                        sectors_left -= 1;
                        let next_sector = sector.wrapping_add(1);
                        self.phase = Phase::Reading {
                            track,
                            sector: next_sector,
                            side,
                            bytes_left: SECTOR_SIZE as u32,
                            offset: 0,
                            sectors_left,
                        };
                        self.delay = 200;
                    } else {
                        self.phase = Phase::FinishingRead;
                        self.delay = 200;
                    }
                } else {
                    self.phase = Phase::Reading {
                        track,
                        sector,
                        side,
                        bytes_left,
                        offset,
                        sectors_left,
                    };
                    self.delay = 128;
                }
            }
            Phase::Writing {
                track,
                sector,
                side,
                bytes_left,
                offset,
                sectors_left,
                buffer,
            } => {
                // Pump byte from data register into buffer. The 8271 expects
                // the CPU to *first* see NDDR (data register ready for the
                // next byte), then write to it. The byte just written by the
                // CPU is currently sitting in self.data.
                let mut buf = buffer;
                if bytes_left == SECTOR_SIZE as u32 {
                    // Brand-new sector: request the very first byte. Don't
                    // consume self.data yet — the CPU hasn't written it.
                    let was_int = self.status & status::INTERRUPT != 0;
                    self.status = status::BUSY | status::NDDR | status::INTERRUPT;
                    if !was_int {
                        self.nmi_pending = true;
                    }
                    self.phase = Phase::Writing {
                        track,
                        sector,
                        side,
                        bytes_left: bytes_left - 1,
                        offset,
                        sectors_left,
                        buffer: buf,
                    };
                    self.delay = 128;
                } else {
                    // Consume the byte the CPU just deposited.
                    buf.push(self.data);
                    let new_left = bytes_left.saturating_sub(1);
                    if new_left == 0 && buf.len() == SECTOR_SIZE {
                        // Sector buffer full — commit to disk.
                        if !self.drives[self.cur_drive].write_sector(side, track, sector, &buf) {
                            self.complete_with(result::WRITE_PROTECTED);
                            return;
                        }
                        if sectors_left > 1 {
                            buf.clear();
                            self.phase = Phase::Writing {
                                track,
                                sector: sector.wrapping_add(1),
                                side,
                                bytes_left: SECTOR_SIZE as u32,
                                offset: 0,
                                sectors_left: sectors_left - 1,
                                buffer: buf,
                            };
                            self.delay = 200;
                        } else {
                            self.phase = Phase::FinishingRead;
                            self.delay = 200;
                        }
                    } else {
                        // More bytes still needed in this sector.
                        let was_int = self.status & status::INTERRUPT != 0;
                        self.status = status::BUSY | status::NDDR | status::INTERRUPT;
                        if !was_int {
                            self.nmi_pending = true;
                        }
                        self.phase = Phase::Writing {
                            track,
                            sector,
                            side,
                            bytes_left: new_left,
                            offset: offset + 1,
                            sectors_left,
                            buffer: buf,
                        };
                        self.delay = 128;
                    }
                }
            }
            Phase::FinishingRead => {
                self.complete_with(result::OK);
            }
            Phase::Complete => {}
        }
    }

    fn complete_with(&mut self, code: u8) {
        self.result = code;
        let was_int = self.status & status::INTERRUPT != 0;
        self.status = status::RESULT_FULL | status::INTERRUPT;
        if !was_int {
            self.nmi_pending = true;
        }
        self.phase = Phase::Complete;
        self.delay = 0;
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        let r = reg & 0x07;
        let value = match r {
            0 => self.status,
            1 => {
                let r = self.result;
                self.status &= !(status::RESULT_FULL | status::INTERRUPT);
                r
            }
            4 | 5 => {
                let d = self.data;
                self.status &= !(status::NDDR | status::INTERRUPT);
                d
            }
            _ => 0,
        };
        if std::env::var("FDC_TRACE_READ").is_ok() {
            eprintln!("FDC R$FE8{r}={value:02X} (phase={:?})", self.phase_kind());
        }
        value
    }

    fn phase_kind(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "Idle",
            Phase::GatherParams { .. } => "GatherParams",
            Phase::Reading { .. } => "Reading",
            Phase::Writing { .. } => "Writing",
            Phase::FinishingRead => "FinishingRead",
            Phase::Complete => "Complete",
        }
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        match reg & 0x07 {
            0 => self.write_command(value),
            1 => self.write_parameter(value),
            2 => {
                // Hardware reset: clears everything except the disk image.
                self.status = 0;
                self.result = 0;
                self.command = 0;
                self.paramnum = 0;
                self.paramreq = 0;
                self.delay = 0;
                self.phase = Phase::Idle;
                self.nmi_pending = false;
            }
            4 | 5 => {
                self.data = value;
                self.status &= !(status::NDDR | status::INTERRUPT);
            }
            _ => {}
        }
    }

    fn write_command(&mut self, value: u8) {
        if self.status & status::BUSY != 0 {
            return; // ignore — chip is busy
        }
        // Bits 6:7 of the command byte select the drive on real hardware.
        self.cur_drive = if value & 0x80 != 0 { 1 } else { 0 };
        self.command = value & 0x3F;
        self.paramnum = 0;
        self.status = status::BUSY;
        self.paramreq = match self.command {
            0x2C => 0,        // Read Drive Status — no parameters
            0x29 | 0x3D => 1, // Seek; Read special register
            0x0A | 0x0E | 0x12 | 0x16 | 0x1E | 0x3A => 2,
            0x0B | 0x0F | 0x13 | 0x17 | 0x1F | 0x1B => 3,
            0x35 => 4, // Specify
            0x00 | 0x04 | 0x23 => 5,
            _ => 0,
        };
        if self.paramreq == 0 {
            self.execute_command();
        } else {
            self.phase = Phase::GatherParams {
                paramreq: self.paramreq,
            };
        }
    }

    fn write_parameter(&mut self, value: u8) {
        if matches!(self.phase, Phase::GatherParams { .. }) {
            if self.paramnum < 5 {
                self.params[self.paramnum as usize] = value;
                self.paramnum += 1;
            }
            if self.paramnum >= self.paramreq {
                self.execute_command();
            }
        }
    }

    fn execute_command(&mut self) {
        if std::env::var("FDC_TRACE").is_ok() {
            eprintln!(
                "FDC cmd=${:02X} params={:02X?} drive={} track={}",
                self.command,
                &self.params[..self.paramreq as usize],
                self.cur_drive,
                self.drives[self.cur_drive].track
            );
        }
        match self.command {
            0x2C => self.cmd_read_drive_status(),
            0x29 => self.cmd_seek(),
            0x35 => self.cmd_specify(),
            0x3D => self.cmd_read_special_register(),
            0x3A => self.cmd_write_special_register(),
            0x1B => self.cmd_read_id(),
            0x12 | 0x13 | 0x16 | 0x17 => self.cmd_read_data(),
            0x0A | 0x0B | 0x0E | 0x0F => self.cmd_write_data(),
            0x1E | 0x1F => self.cmd_verify_data(),
            0x23 => self.cmd_format_track(),
            _ => self.complete_with(result::SECTOR_NOT_FOUND),
        }
    }

    fn cmd_read_drive_status(&mut self) {
        // 8271 "drive in" byte. Layout per beebjit / jsbeeb intel-fdc.js
        // (cross-checked against scarybeasts' real-hardware observations):
        //   bit 7  always 1            (sentinel)
        //   bit 6  RDY1   — drive 1 selected & spinning
        //   bit 5  unused
        //   bit 4  INDEX  — index pulse (we model as always-low)
        //   bit 3  WR_PROT
        //   bit 2  RDY0   — drive 0 selected & spinning
        //   bit 1  TRK0   — head at track 0 of selected drive
        //   bit 0  always 1            (sentinel)
        //
        // DFS-098 checks the sentinels and the RDY/TRK0 bits when deciding
        // whether to issue Read Special Register / additional Read Drive
        // Status calls before kicking off a Read Data. Missing the bit-0
        // sentinel diverts DFS down the wrong branch and it never seeks to
        // the file's track.
        let mut s: u8 = 0x81;
        // The RDY/TRK0/INDEX bits only assert while the drive is actually
        // spinning, i.e. the corresponding select bit + the loadHead bit are
        // set in mmioDriveOut ($23). DFS uses the absence-then-presence of
        // RDY0 as a "drive newly ready" trigger; always reporting it ready
        // makes DFS skip the second Read Drive Status / Seek pair, which in
        // turn leaves the post-catalogue state machine half-initialised.
        let drive_out = self.special[0x23];
        let select_0 = drive_out & 0x40 != 0;
        let select_1 = drive_out & 0x80 != 0;
        let load_head = drive_out & 0x08 != 0;
        let stable = self.spin_up_remaining == 0;
        let spinning_0 = select_0 && load_head && self.drives[0].loaded() && stable;
        let spinning_1 = select_1 && load_head && self.drives[1].loaded() && stable;
        if self.drives[self.cur_drive].write_protect {
            s |= 0x08;
        }
        if spinning_0 {
            s |= 0x04;
        }
        if spinning_1 {
            s |= 0x40;
        }
        if (spinning_0 || spinning_1) && self.drives[self.cur_drive].track == 0 {
            s |= 0x02;
        }
        if (spinning_0 || spinning_1) && self.index_active() {
            s |= 0x10;
        }
        self.result = s;
        self.status = status::RESULT_FULL;
        // No NMI for Read Drive Status — the result register is just polled.
        self.phase = Phase::Idle;
    }

    fn cmd_seek(&mut self) {
        let target = self.params[0];
        if target as usize >= TRACKS_PER_DISK {
            self.complete_with(result::SECTOR_NOT_FOUND);
            return;
        }
        self.drives[self.cur_drive].track = target;
        // Approximate step time: rough 3 ms per track at 2 MHz = 6000 cycles per track.
        let prev = self.drives[self.cur_drive].track;
        let delta = (target as i16 - prev as i16).unsigned_abs() as u32;
        self.delay = (delta * (self.step_rate as u32 + 1) * 100).max(200);
        self.complete_with(result::OK);
    }

    fn cmd_specify(&mut self) {
        // Two specify sub-commands distinguished by params[0]:
        //   0x0D: Initialise — params[1..4] = step, settle, head-load
        //   0x10: Load bad tracks for surface 0 (drive 0)
        //   0x18: Load bad tracks for surface 1 (drive 1)
        match self.params[0] {
            0x0D => {
                self.step_rate = self.params[1];
                self.head_settle = self.params[2];
                self.head_load = self.params[3];
            }
            0x10 => {
                self.special[0] = self.params[1];
                self.special[1] = self.params[2];
                self.special[2] = self.params[3];
                self.special[3] = self.params[4];
            }
            0x18 => {
                self.special[10] = self.params[1];
                self.special[11] = self.params[2];
                self.special[12] = self.params[3];
                self.special[13] = self.params[4];
            }
            _ => {}
        }
        self.complete_with(result::OK);
    }

    fn cmd_read_special_register(&mut self) {
        let idx = self.params[0] & 0x3F;
        self.result = self.special[idx as usize];
        self.status = status::RESULT_FULL;
        self.phase = Phase::Idle;
    }

    fn cmd_write_special_register(&mut self) {
        let idx = self.params[0] & 0x3F;
        let val = self.params[1];
        // Register $23 is `mmioDriveOut` — DFS writes here to select a drive
        // and load the head. Bits per jsbeeb's `DriveOut` enum:
        //   $80 select_1, $40 select_0, $20 side, $08 loadHead, $01 writeEnable.
        // We mirror the select bits into cur_drive / side so subsequent
        // Read Drive Status / Seek / Read Data see the right drive.
        if idx == 0x23 {
            let prev = self.special[0x23];
            let was_spinning = (prev & 0xC0) != 0 && (prev & 0x08) != 0;
            let now_spinning = (val & 0xC0) != 0 && (val & 0x08) != 0;
            match val & 0xC0 {
                0x40 => self.cur_drive = 0,
                0x80 => self.cur_drive = 1,
                _ => {} // both / neither — leave cur_drive alone
            }
            self.side = if val & 0x20 != 0 { 1 } else { 0 };
            if now_spinning && !was_spinning {
                // Drive newly spun up — block RDY/INDEX for ~0.5 ms so DFS
                // observes the not-ready → ready transition across two
                // consecutive Read Drive Status polls.
                self.spin_up_remaining = 1_000;
            }
        }
        self.special[idx as usize] = val;
        self.complete_with(result::OK);
    }

    fn cmd_read_id(&mut self) {
        // Read ID returns 4 result bytes (track, head, sector, length). For
        // a normal SSD all sectors are 256 bytes (size code = 1). The 8271
        // delivers these bytes via the data register one at a time.
        let track = self.drives[self.cur_drive].track;
        if track as usize >= TRACKS_PER_DISK {
            self.complete_with(result::SECTOR_NOT_FOUND);
            return;
        }
        // We model Read ID as a 4-byte sequence using the read pump but with
        // a synthetic "sector" of 4 bytes. Real hardware reads the address
        // mark off the disc; our SSD images don't carry headers, so we
        // synthesise them.
        let _ = self.params; // params[2] = sectors (we ignore)
        // Simplest acceptable behaviour: directly fill data register with
        // track number and signal completion.
        self.data = track;
        self.status = status::RESULT_FULL | status::NDDR;
        self.nmi_pending = true;
        self.complete_with(result::OK);
    }

    fn cmd_read_data(&mut self) {
        let track = self.params[0];
        let sector = self.params[1];
        let count = (self.params[2] & 0x1F).max(1);
        if track as usize >= TRACKS_PER_DISK || sector as usize >= SECTORS_PER_TRACK {
            self.complete_with(result::SECTOR_NOT_FOUND);
            return;
        }
        self.drives[self.cur_drive].track = track;
        self.phase = Phase::Reading {
            track,
            sector,
            side: self.side,
            bytes_left: SECTOR_SIZE as u32,
            offset: 0,
            sectors_left: count,
        };
        // Real 8271 waits for the sector ID to come round under the head
        // before delivering the first byte — up to one full revolution
        // (~400_000 cycles at 2 MHz). DFS uses this latency to install the
        // NMI buffer pointer ($A6/$A7) and the byte counter ($A3/$A4/$A5).
        // 8_000 cycles (≈4 ms) is enough for the slowest published DFS-098
        // / DFS-090 setup path; faster would race the CPU and lose bytes.
        self.delay = 8_000;
    }

    fn cmd_write_data(&mut self) {
        let track = self.params[0];
        let sector = self.params[1];
        let count = (self.params[2] & 0x1F).max(1);
        if track as usize >= TRACKS_PER_DISK || sector as usize >= SECTORS_PER_TRACK {
            self.complete_with(result::SECTOR_NOT_FOUND);
            return;
        }
        if self.drives[self.cur_drive].write_protect {
            self.complete_with(result::WRITE_PROTECTED);
            return;
        }
        self.drives[self.cur_drive].track = track;
        self.phase = Phase::Writing {
            track,
            sector,
            side: self.side,
            bytes_left: SECTOR_SIZE as u32,
            offset: 0,
            sectors_left: count,
            buffer: Vec::with_capacity(SECTOR_SIZE),
        };
        // Head settle, then request the first byte (handled in advance_phase
        // when bytes_left == SECTOR_SIZE — special "request first byte"
        // branch fires the NDDR NMI then).
        self.status = status::BUSY;
        self.delay = 200;
    }

    fn cmd_verify_data(&mut self) {
        // Implemented as a successful read-without-store. We don't compute
        // CRCs from the SSD image (it has none), so any sector that exists
        // verifies OK.
        let track = self.params[0];
        let sector = self.params[1];
        if track as usize >= TRACKS_PER_DISK || sector as usize >= SECTORS_PER_TRACK {
            self.complete_with(result::SECTOR_NOT_FOUND);
            return;
        }
        self.complete_with(result::OK);
    }

    fn cmd_format_track(&mut self) {
        // Format zeroes all sectors of the addressed track. SSD images don't
        // store track-level metadata so we just zero the bytes.
        let track = self.params[0];
        if track as usize >= TRACKS_PER_DISK {
            self.complete_with(result::SECTOR_NOT_FOUND);
            return;
        }
        for sector in 0..SECTORS_PER_TRACK {
            let blank = [0u8; SECTOR_SIZE];
            let _ =
                self.drives[self.cur_drive].write_sector(self.side, track, sector as u8, &blank);
        }
        self.complete_with(result::OK);
    }

    pub fn status_byte(&self) -> u8 {
        self.status
    }

    pub fn drive_count(&self) -> usize {
        self.drives.iter().filter(|d| d.loaded()).count()
    }

    pub fn current_track(&self) -> u8 {
        self.drives[self.cur_drive].track
    }

    // ---- Snapshot accessors ----
    pub fn drive_track(&self, drive: usize) -> u8 {
        self.drives.get(drive).map(|d| d.track).unwrap_or(0)
    }
    pub fn set_drive_track(&mut self, drive: usize, track: u8) {
        if let Some(d) = self.drives.get_mut(drive) {
            d.track = track;
        }
    }
    pub fn cur_drive_index(&self) -> usize {
        self.cur_drive
    }
    pub fn set_cur_drive(&mut self, drive: usize) {
        if drive < 2 {
            self.cur_drive = drive;
        }
    }
    pub fn side_index(&self) -> u8 {
        self.side
    }
    pub fn set_side(&mut self, side: u8) {
        self.side = side & 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_ssd() -> Vec<u8> {
        // 200 KiB, sector i,j filled with i and j for verification.
        let mut img = vec![0u8; SSD_SIZE];
        for t in 0..TRACKS_PER_DISK {
            for s in 0..SECTORS_PER_TRACK {
                let off = (t * SECTORS_PER_TRACK + s) * SECTOR_SIZE;
                img[off] = t as u8;
                img[off + 1] = s as u8;
            }
        }
        img
    }

    #[test]
    fn fdc_loads_ssd_and_reports_correct_size() {
        let mut fdc = Fdc8271::new();
        fdc.load_image(0, dummy_ssd()).unwrap();
        assert!(fdc.has_disk());
        assert_eq!(fdc.drive_count(), 1);
    }

    #[test]
    fn read_drive_status_reflects_loaded_image_on_drive_0() {
        let mut fdc = Fdc8271::new();
        fdc.load_image(0, dummy_ssd()).unwrap();
        // Spin drive 0 up by setting mmioDriveOut ($23) = select_0 | loadHead.
        fdc.write(0, 0x3A);
        fdc.write(1, 0x23);
        fdc.write(1, 0x48);
        // Wait past the spin-up window so RDY/TRK0/INDEX become observable.
        fdc.tick(2_000);
        fdc.write(0, 0x2C);
        assert_eq!(fdc.status_byte() & status::RESULT_FULL, status::RESULT_FULL);
        let r = fdc.read(1);
        assert!(r & 0x80 != 0, "bit 7 sentinel should be set");
        assert!(r & 0x04 != 0, "RDY0 should be set once drive selected");
        assert!(r & 0x02 != 0, "TRK0 should be set at track 0");
        assert!(r & 0x01 != 0, "bit 0 sentinel should be set");
    }

    #[test]
    fn seek_updates_current_track() {
        let mut fdc = Fdc8271::new();
        fdc.load_image(0, dummy_ssd()).unwrap();
        fdc.write(0, 0x29); // Seek
        fdc.write(1, 17); // track 17
        assert_eq!(fdc.current_track(), 17);
    }

    #[test]
    fn read_data_streams_bytes_via_nmi() {
        let mut fdc = Fdc8271::new();
        fdc.load_image(0, dummy_ssd()).unwrap();
        fdc.write(0, 0x13); // Read Data, multi-sector
        fdc.write(1, 5); // track
        fdc.write(1, 3); // sector
        fdc.write(1, 1); // 1 sector
        // First byte appears after the head-settle / sector-search delay
        // (≈8000 cycles in our model — matches DFS's expectation that it has
        // time to install the NMI buffer pointer between Read Data issue and
        // the first byte arriving).
        fdc.tick(8500);
        assert!(fdc.poll_nmi_edge());
        assert_eq!(fdc.read(4), 5);
        // Next byte follows after the inter-byte delay (64 cycles between
        // bytes within a sector).
        fdc.tick(150);
        assert!(fdc.poll_nmi_edge());
        assert_eq!(fdc.read(4), 3);
    }

    #[test]
    fn oversized_image_is_rejected() {
        let mut fdc = Fdc8271::new();
        // Anything larger than SSD but not exactly DSD must be refused.
        let err = fdc.load_image(0, vec![0u8; SSD_SIZE + 1]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains(&format!("{}", SSD_SIZE + 1)));
    }

    #[test]
    fn undersized_image_is_padded_and_loads_single_sided() {
        let mut fdc = Fdc8271::new();
        // Real DFS images can be shorter than 200 KiB if trailing tracks are
        // empty; we accept them and zero-pad up to the full SSD size.
        fdc.load_image(0, vec![0xAAu8; 12345]).unwrap();
        assert!(fdc.has_disk());
        // First few bytes are still 0xAA, the tail is the 0-padding.
        let head = fdc.drives[0].read_sector(0, 0, 0).unwrap();
        assert_eq!(head[0], 0xAA);
    }

    #[test]
    fn dsd_image_loads_as_double_sided() {
        let mut fdc = Fdc8271::new();
        fdc.load_image(0, vec![0u8; DSD_SIZE]).unwrap();
        assert!(fdc.has_disk());
    }
}
