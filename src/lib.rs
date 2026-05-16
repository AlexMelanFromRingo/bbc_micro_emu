//! BBC Micro Model B emulator.
//!
//! Memory map (Model B with single language ROM and OS ROM):
//!
//! ```text
//!   $0000-$7FFF   32 KiB main RAM
//!   $8000-$BFFF   16 KiB paged "sideways" ROM (1-16 banks, selected by $FE30)
//!   $C000-$FBFF   15.75 KiB MOS (OS) ROM
//!   $FC00-$FCFF   FRED — external 1 MHz expansion (not implemented)
//!   $FD00-$FDFF   JIM  — external 1 MHz expansion (not implemented)
//!   $FE00-$FEFF   SHEILA — internal memory-mapped I/O
//!   $FF00-$FFFF   top of MOS ROM (NMI/RESET/IRQ vectors live here)
//! ```
//!
//! SHEILA sub-map (every device is mirrored on a 32-byte stride):
//!
//! ```text
//!   $FE00-$FE07   6845 CRTC
//!   $FE08-$FE0F   6850 ACIA serial
//!   $FE10-$FE1F   Serial ULA + cassette
//!   $FE20-$FE2F   Video ULA
//!   $FE30-$FE3F   paged ROM select (write-only)
//!   $FE40-$FE5F   System 6522 VIA
//!   $FE60-$FE7F   User   6522 VIA
//!   $FE80-$FE9F   8271 or WD1770 floppy disc controller
//!   $FEA0-$FEBF   68B54 Econet ADLC
//!   $FEC0-$FEDF   uPD7002 analogue/digital converter
//!   $FEE0-$FEFF   Tube ULA
//! ```

pub mod acia6850;
pub mod bus;
pub mod crtc;
pub mod display;
pub mod fdc8271;
pub mod hardware;
pub mod machine;
pub mod memory;
pub mod renderer;
pub mod sheila;
pub mod sn76489;
pub mod system_via;
pub mod upd7002;
pub mod user_via;
pub mod via6522;
pub mod video_ula;

pub use acia6850::{Acia6850, SerialUla};
pub use bus::BbcBus;
pub use crtc::{Crtc6845, CrtcEvents};
pub use fdc8271::Fdc8271;
pub use hardware::{AccessLog, Hardware};
pub use machine::{Machine, MachineConfig, MachineError};
pub use memory::{Memory, MemoryConfig, RomBank, RomLoadError};
pub use renderer::{Framebuffer, Renderer, SCREEN_H, SCREEN_W};
pub use sheila::SheilaDevice;
pub use sn76489::Sn76489;
pub use system_via::{BbcKey, SystemVia};
pub use upd7002::UpD7002;
pub use user_via::UserVia;
pub use via6522::Via6522;
pub use video_ula::VideoUla;
