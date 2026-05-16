//! Minimal binary save / load of the running machine state.
//!
//! Format (little-endian, version 1):
//!
//! ```text
//! offset  size  field
//! ------  ----  -----------------------------------------------------------
//!     0     4   magic = b"BBC1"
//!     4     1   format version (1)
//!     5     1   selected_bank
//!     6     1   reserved (= 0)
//!     7     1   reserved (= 0)
//!     8     8   cpu.cycles (u64)
//!    16     1   cpu.a
//!    17     1   cpu.x
//!    18     1   cpu.y
//!    19     1   cpu.sp
//!    20     1   cpu.status
//!    21     1   reserved
//!    22     2   cpu.pc
//!    24     2   crtc.scanline_in_frame
//!    26     2   crtc.cycle_in_scanline
//!    28    18   crtc.regs[0..18]
//!    46     1   video_ula.control
//!    47     1   video_ula.screen_size_code
//!    48    16   video_ula.palette
//!    64     1   system_via.ic32
//!    65    10   system_via.keys[0..10]
//!    75     5   reserved
//!    80    32   sn76489 — per-channel period (u16 LE)×4 + atten (u8)×4 +
//!               noise_ctrl (u8) + 15 reserved
//!   112  varies fdc — drive 0 current track (u8) + drive 1 current track
//!               (u8) + cur_drive (u8) + side (u8)
//!   116     0   end of header
//!   116 32768   ram[0..32768]
//! ```
//!
//! Disk images, ROMs, audio stream and live winit/cpal threads are NOT
//! included. The expected workflow is "rebuild a fresh Machine with the
//! same `MachineConfig`, mount the same disc, then call `load_state`".

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::machine::Machine;

pub const MAGIC: &[u8; 4] = b"BBC1";
pub const VERSION: u8 = 1;
pub const RAM_OFFSET: usize = 116;
pub const SNAPSHOT_LEN: usize = RAM_OFFSET + crate::memory::RAM_SIZE;

#[derive(Debug)]
pub enum SnapshotError {
    Io(io::Error),
    BadMagic,
    BadVersion(u8),
    WrongSize { expected: usize, actual: usize },
}

impl From<io::Error> for SnapshotError {
    fn from(e: io::Error) -> Self {
        SnapshotError::Io(e)
    }
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::BadMagic => write!(f, "not a BBC Micro snapshot (bad magic)"),
            Self::BadVersion(v) => write!(f, "unsupported snapshot version {v}"),
            Self::WrongSize { expected, actual } => write!(
                f,
                "snapshot wrong size: expected {expected} bytes, got {actual}"
            ),
        }
    }
}

impl std::error::Error for SnapshotError {}

pub fn save_to_path(machine: &Machine, path: &Path) -> Result<(), SnapshotError> {
    let mut f = File::create(path)?;
    f.write_all(&encode(machine))?;
    Ok(())
}

pub fn load_from_path(machine: &mut Machine, path: &Path) -> Result<(), SnapshotError> {
    let mut buf = Vec::new();
    File::open(path)?.read_to_end(&mut buf)?;
    decode(machine, &buf)
}

pub fn encode(machine: &Machine) -> Vec<u8> {
    let mut buf = vec![0u8; SNAPSHOT_LEN];
    buf[0..4].copy_from_slice(MAGIC);
    buf[4] = VERSION;
    buf[5] = machine.bus.memory.selected_bank();
    let cpu = &machine.cpu;
    buf[8..16].copy_from_slice(&cpu.cycles.to_le_bytes());
    buf[16] = cpu.registers.a;
    buf[17] = cpu.registers.x;
    buf[18] = cpu.registers.y;
    buf[19] = cpu.registers.sp;
    buf[20] = cpu.registers.status;
    buf[22..24].copy_from_slice(&cpu.registers.pc.to_le_bytes());
    let crtc = &machine.bus.hardware.crtc;
    buf[24..26].copy_from_slice(&crtc.scanline_in_frame.to_le_bytes());
    buf[26..28].copy_from_slice(&crtc.cycle_in_scanline.to_le_bytes());
    for (i, &r) in crtc.regs_slice().iter().enumerate() {
        buf[28 + i] = r;
    }
    let ula = &machine.bus.hardware.video_ula;
    buf[46] = ula.control;
    buf[47] = ula.screen_size_code;
    buf[48..64].copy_from_slice(&ula.palette);
    buf[64] = machine.bus.hardware.system_via.ic32;
    buf[65..75].copy_from_slice(&machine.bus.hardware.system_via.keys);
    if let Ok(snd) = machine.bus.hardware.sound.lock() {
        for c in 0..4 {
            let off = 80 + c * 2;
            buf[off..off + 2].copy_from_slice(&snd.channel_period(c).to_le_bytes());
            buf[88 + c] = snd.channel_attenuation(c);
        }
        buf[92] = snd.noise_ctrl_byte();
    }
    let fdc = &machine.bus.hardware.fdc;
    buf[112] = fdc.drive_track(0);
    buf[113] = fdc.drive_track(1);
    buf[114] = fdc.cur_drive_index() as u8;
    buf[115] = fdc.side_index();
    buf[RAM_OFFSET..].copy_from_slice(machine.bus.memory.ram());
    buf
}

pub fn decode(machine: &mut Machine, buf: &[u8]) -> Result<(), SnapshotError> {
    if buf.len() < SNAPSHOT_LEN {
        return Err(SnapshotError::WrongSize {
            expected: SNAPSHOT_LEN,
            actual: buf.len(),
        });
    }
    if &buf[0..4] != MAGIC {
        return Err(SnapshotError::BadMagic);
    }
    if buf[4] != VERSION {
        return Err(SnapshotError::BadVersion(buf[4]));
    }
    machine.bus.memory.select_bank(buf[5]);
    machine.cpu.cycles = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    machine.cpu.registers.a = buf[16];
    machine.cpu.registers.x = buf[17];
    machine.cpu.registers.y = buf[18];
    machine.cpu.registers.sp = buf[19];
    machine.cpu.registers.status = buf[20];
    machine.cpu.registers.pc = u16::from_le_bytes(buf[22..24].try_into().unwrap());
    let crtc = &mut machine.bus.hardware.crtc;
    crtc.scanline_in_frame = u16::from_le_bytes(buf[24..26].try_into().unwrap());
    crtc.cycle_in_scanline = u16::from_le_bytes(buf[26..28].try_into().unwrap());
    let regs = crtc.regs_slice_mut();
    for (i, slot) in regs.iter_mut().enumerate() {
        *slot = buf[28 + i];
    }
    let ula = &mut machine.bus.hardware.video_ula;
    ula.control = buf[46];
    ula.screen_size_code = buf[47];
    ula.palette.copy_from_slice(&buf[48..64]);
    ula.reset_per_scanline();
    machine.bus.hardware.system_via.ic32 = buf[64];
    machine
        .bus
        .hardware
        .system_via
        .keys
        .copy_from_slice(&buf[65..75]);
    if let Ok(mut snd) = machine.bus.hardware.sound.lock() {
        for c in 0..4 {
            let off = 80 + c * 2;
            let period = u16::from_le_bytes(buf[off..off + 2].try_into().unwrap());
            snd.set_channel_period(c, period);
            snd.set_channel_attenuation(c, buf[88 + c]);
        }
        snd.set_noise_ctrl_byte(buf[92]);
    }
    let fdc = &mut machine.bus.hardware.fdc;
    fdc.set_drive_track(0, buf[112]);
    fdc.set_drive_track(1, buf[113]);
    fdc.set_cur_drive(buf[114] as usize);
    fdc.set_side(buf[115]);
    machine
        .bus
        .memory
        .ram_mut()
        .copy_from_slice(&buf[RAM_OFFSET..]);
    Ok(())
}
