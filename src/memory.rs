//! Memory subsystem: 32K RAM, 16 paged ROM banks at $8000-$BFFF, MOS ROM at $C000-$FFFF.
//!
//! ROM bank selection is via `$FE30` (write-only, latch lower 4 bits = bank index on
//! Model B issue 4+; the upper nibble is unused). On real hardware the selection
//! register is part of the address decoder, but for clarity it lives here.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

pub const RAM_SIZE: usize = 0x8000;
pub const ROM_BANK_SIZE: usize = 0x4000;
pub const MOS_ROM_SIZE: usize = 0x4000;
pub const MAX_ROM_BANKS: usize = 16;

pub const SIDEWAYS_BASE: u16 = 0x8000;
pub const MOS_BASE: u16 = 0xC000;

#[derive(Debug)]
pub enum RomLoadError {
    Io {
        path: PathBuf,
        err: std::io::Error,
    },
    BadSize {
        path: PathBuf,
        expected: usize,
        actual: usize,
    },
    BankOutOfRange {
        bank: u8,
    },
}

impl Display for RomLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, err } => write!(f, "cannot read ROM `{}`: {err}", path.display()),
            Self::BadSize {
                path,
                expected,
                actual,
            } => write!(
                f,
                "ROM `{}` has unexpected size: expected {expected} bytes, got {actual}",
                path.display()
            ),
            Self::BankOutOfRange { bank } => write!(
                f,
                "sideways ROM bank {bank} is out of range (max {})",
                MAX_ROM_BANKS - 1
            ),
        }
    }
}

impl Error for RomLoadError {}

/// One 16 KiB paged ROM slot. An empty slot reads as $FF (open bus on real hw).
#[derive(Clone)]
pub struct RomBank {
    data: Option<Box<[u8; ROM_BANK_SIZE]>>,
}

impl RomBank {
    pub fn empty() -> Self {
        Self { data: None }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RomLoadError> {
        Self::from_bytes_at(bytes, &PathBuf::from("<inline>"))
    }

    fn from_bytes_at(bytes: &[u8], path: &Path) -> Result<Self, RomLoadError> {
        let mut buf = Box::new([0u8; ROM_BANK_SIZE]);
        // Accept 16 KiB ROMs (full bank), 8 KiB ROMs (mirrored — BBC issue 4
        // socket doesn't decode A13), and 4 KiB ROMs (mirrored four times,
        // used by some peripheral chips). Anything else is an error.
        match bytes.len() {
            ROM_BANK_SIZE => buf.copy_from_slice(bytes),
            0x2000 => {
                buf[..0x2000].copy_from_slice(bytes);
                buf[0x2000..].copy_from_slice(bytes);
            }
            0x1000 => {
                for chunk in buf.chunks_mut(0x1000) {
                    chunk.copy_from_slice(bytes);
                }
            }
            actual => {
                return Err(RomLoadError::BadSize {
                    path: path.to_path_buf(),
                    expected: ROM_BANK_SIZE,
                    actual,
                });
            }
        }
        Ok(Self { data: Some(buf) })
    }

    pub fn from_file(path: &Path) -> Result<Self, RomLoadError> {
        let bytes = std::fs::read(path).map_err(|err| RomLoadError::Io {
            path: path.to_path_buf(),
            err,
        })?;
        Self::from_bytes_at(&bytes, path)
    }

    #[inline]
    pub fn read(&self, offset: u16) -> u8 {
        match &self.data {
            Some(buf) => buf[offset as usize],
            None => 0xFF,
        }
    }

    pub fn is_present(&self) -> bool {
        self.data.is_some()
    }
}

#[derive(Default)]
pub struct MemoryConfig {
    pub mos_rom_path: Option<PathBuf>,
    pub mos_rom_bytes: Option<Vec<u8>>,
    /// Paged ROM banks. Index 0 is the lowest priority on real hardware; on power-up
    /// the highest-numbered present bank is selected by MOS during init.
    pub rom_banks: [Option<PathBuf>; MAX_ROM_BANKS],
    /// Bank index selected at power-up. MOS will overwrite this during init.
    pub initial_bank: u8,
}

pub struct Memory {
    ram: Box<[u8; RAM_SIZE]>,
    mos: Box<[u8; MOS_ROM_SIZE]>,
    banks: [RomBank; MAX_ROM_BANKS],
    selected_bank: u8,
}

impl Memory {
    pub fn new(config: MemoryConfig) -> Result<Self, RomLoadError> {
        let mut mos = Box::new([0xFFu8; MOS_ROM_SIZE]);

        if let Some(bytes) = config.mos_rom_bytes.as_ref() {
            if bytes.len() != MOS_ROM_SIZE {
                return Err(RomLoadError::BadSize {
                    path: PathBuf::from("<mos:inline>"),
                    expected: MOS_ROM_SIZE,
                    actual: bytes.len(),
                });
            }
            mos.copy_from_slice(bytes);
        } else if let Some(path) = config.mos_rom_path.as_ref() {
            let bytes = std::fs::read(path).map_err(|err| RomLoadError::Io {
                path: path.clone(),
                err,
            })?;
            if bytes.len() != MOS_ROM_SIZE {
                return Err(RomLoadError::BadSize {
                    path: path.clone(),
                    expected: MOS_ROM_SIZE,
                    actual: bytes.len(),
                });
            }
            mos.copy_from_slice(&bytes);
        }
        // else: leave MOS area filled with $FF — the CPU will reset to ($FFFC) and
        // immediately read $FFFF, which is fine for unit-testing without a real OS ROM.

        let mut banks: [RomBank; MAX_ROM_BANKS] = std::array::from_fn(|_| RomBank::empty());
        for (i, slot) in config.rom_banks.iter().enumerate() {
            if let Some(path) = slot {
                banks[i] = RomBank::from_file(path)?;
            }
        }

        let initial_bank = config.initial_bank & 0x0F;

        // Real BBC DRAM is undefined at power-on; jsbeeb (and most
        // emulators) initialise it to $FF, matching what unpowered DRAM
        // settles to after a long power-off. DFS-090 relies on this:
        // `$83E1`'s stack-manipulation RTS lands at `$0384` and expects
        // whatever's there to act as a no-op trampoline, which $FF $FF $FF
        // (= ISC $FFFF,X) effectively is for our purposes — `$00` (BRK)
        // would fault into the error handler.
        Ok(Self {
            ram: Box::new([0xFFu8; RAM_SIZE]),
            mos,
            banks,
            selected_bank: initial_bank,
        })
    }

    pub fn install_bank(&mut self, bank: u8, rom: RomBank) -> Result<(), RomLoadError> {
        let idx = bank as usize;
        if idx >= MAX_ROM_BANKS {
            return Err(RomLoadError::BankOutOfRange { bank });
        }
        self.banks[idx] = rom;
        Ok(())
    }

    pub fn ram(&self) -> &[u8] {
        self.ram.as_ref()
    }

    pub fn ram_mut(&mut self) -> &mut [u8] {
        self.ram.as_mut()
    }

    pub fn mos_byte(&self, addr: u16) -> u8 {
        debug_assert!(addr >= MOS_BASE);
        self.mos[(addr - MOS_BASE) as usize]
    }

    pub fn selected_bank(&self) -> u8 {
        self.selected_bank
    }

    pub fn select_bank(&mut self, bank: u8) {
        self.selected_bank = bank & 0x0F;
    }

    pub fn bank_is_present(&self, bank: u8) -> bool {
        (bank as usize) < MAX_ROM_BANKS && self.banks[bank as usize].is_present()
    }

    /// CPU read at `addr` (no side effects on memory itself).
    #[inline]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.ram[addr as usize],
            0x8000..=0xBFFF => self.banks[self.selected_bank as usize].read(addr - SIDEWAYS_BASE),
            0xC000..=0xFFFF => self.mos[(addr - MOS_BASE) as usize],
        }
    }

    /// CPU write at `addr`. Writes into ROM regions are silently dropped (open bus).
    #[inline]
    pub fn write(&mut self, addr: u16, value: u8) {
        if (addr as usize) < RAM_SIZE {
            self.ram[addr as usize] = value;
        }
        // writes to $8000-$FFFF (ROM) are ignored.
    }
}
