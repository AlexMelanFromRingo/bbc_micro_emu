//! Motorola 6850 ACIA (Asynchronous Communications Interface Adapter).
//!
//! In the BBC Micro the 6850 sits at $FE08 (CR/SR) and $FE09 (TDR/RDR). It's
//! used for the RS-423 serial port AND for cassette I/O — the Serial ULA at
//! $FE10 routes one or the other to the ACIA's RX/TX lines. CTS/DCD on the
//! ACIA come from the cassette read-data line / RS-423 hardware.
//!
//! Register map (MOS programs the 6850 in standard 8N1 mode):
//!
//! ```text
//!   $FE08 W = control register (CR)
//!   $FE08 R = status register  (SR)
//!   $FE09 W = transmit data register (TDR)
//!   $FE09 R = receive data register  (RDR)
//! ```
//!
//! Control register layout:
//!
//! ```text
//!   bit 1:0  Master clock divide
//!              00 = ÷ 1
//!              01 = ÷ 16
//!              10 = ÷ 64
//!              11 = master reset
//!   bit 4:2  Word select (parity + stop bits + word length)
//!   bit 6:5  Transmitter control
//!              00 = TX interrupt disabled, /RTS low
//!              01 = TX interrupt enabled,  /RTS low
//!              10 = TX interrupt disabled, /RTS high
//!              11 = TX interrupt disabled, /RTS low, BREAK on TX
//!   bit 7    Receiver interrupt enable (1 = enable RDRF / DCD interrupts)
//! ```
//!
//! Status register layout:
//!
//! ```text
//!   bit 0  RDRF — Receive Data Register Full
//!   bit 1  TDRE — Transmit Data Register Empty
//!   bit 2  /DCD — Data Carrier Detect (input pin state)
//!   bit 3  /CTS — Clear To Send (input pin state)
//!   bit 4  FE   — Framing Error
//!   bit 5  OVRN — Receiver Overrun
//!   bit 6  PE   — Parity Error
//!   bit 7  IRQ  — Interrupt Request
//! ```

#[derive(Debug, Default)]
pub struct Acia6850 {
    /// Control register value (last write to $FE08).
    pub control: u8,
    /// Status register value (computed on read).
    status: u8,
    /// Receive data register (most recent byte from RX line).
    rdr: u8,
    /// Transmit data register (most recent byte written by CPU).
    tdr: u8,
    /// Input lines /DCD and /CTS modelled as booleans (true = asserted = low).
    pub dcd_asserted: bool,
    pub cts_asserted: bool,
    /// IRQ output (output of the ACIA OR-tied with other devices).
    pub irq: bool,
}

impl Acia6850 {
    pub fn new() -> Self {
        let mut a = Self::default();
        a.master_reset();
        a
    }

    fn master_reset(&mut self) {
        // Per Motorola MC6850 datasheet, master reset clears the status
        // register and any pending interrupts but leaves the transmit and
        // receive data registers untouched. The transmitter is forced
        // active-low (TDRE set so the CPU may immediately load a byte).
        self.status = 0x02; // TDRE
        self.irq = false;
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 0x01 {
            0 => {
                // Status register read returns the current status. Reading
                // does not clear flags — those clear on subsequent reads of
                // RDR (for RDRF) or writes of TDR (for TDRE).
                let mut s = self.status;
                if self.cts_asserted {
                    s |= 0x08;
                }
                if self.dcd_asserted {
                    s |= 0x04;
                }
                if self.irq {
                    s |= 0x80;
                }
                s
            }
            _ => {
                // Reading RDR clears RDRF / OVRN / FE / PE / IRQ-from-RX.
                let v = self.rdr;
                self.status &= !(0x01 | 0x10 | 0x20 | 0x40);
                self.update_irq();
                v
            }
        }
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        match reg & 0x01 {
            0 => {
                if (value & 0x03) == 0x03 {
                    self.master_reset();
                }
                self.control = value;
                self.update_irq();
            }
            _ => {
                self.tdr = value;
                // Writing TDR clears TDRE until the transmit-shift register
                // pulls the byte out (modelled instantaneously here).
                self.status &= !0x02;
                // Immediately mark TDRE again — we never block the CPU.
                self.status |= 0x02;
                self.update_irq();
            }
        }
    }

    /// Feed a byte to the receiver (called by the cassette / RS-423 backend).
    pub fn receive(&mut self, byte: u8) {
        if self.status & 0x01 != 0 {
            // Existing byte not read → overrun.
            self.status |= 0x20;
        }
        self.rdr = byte;
        self.status |= 0x01; // RDRF
        self.update_irq();
    }

    pub fn poll_tx(&mut self) -> Option<u8> {
        // Returns the byte to be transmitted. In our model we always have a
        // ready byte after a TDR write; we return the last-written value once
        // (a real chip's TX shift register).
        let byte = self.tdr;
        Some(byte)
    }

    fn update_irq(&mut self) {
        let rx_irq = (self.control & 0x80 != 0) && (self.status & 0x01 != 0);
        let tx_irq = match (self.control >> 5) & 0x03 {
            0b01 => self.status & 0x02 != 0,
            _ => false,
        };
        self.irq = rx_irq || tx_irq;
    }

    pub fn poll_irq(&self) -> bool {
        self.irq
    }
}

/// Acorn Serial ULA at $FE10 — routes RX/TX between the cassette and the
/// RS-423 connector and selects baud-rate dividers.
///
/// Register layout (write-only):
///
/// ```text
///   bit 7   Motor relay (1 = motor on)
///   bit 6   Cassette / RS-423 select (1 = cassette)
///   bit 5:3 RX rate select  (000=19200, 001=1200, 010=4800, 011=150,
///                            100=9600,  101=300,  110=2400,  111=75)
///   bit 2:0 TX rate select  (same encoding)
/// ```
#[derive(Default, Debug, Clone, Copy)]
pub struct SerialUla {
    pub control: u8,
}

impl SerialUla {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, value: u8) {
        self.control = value;
    }

    pub fn motor_on(&self) -> bool {
        self.control & 0x80 != 0
    }

    pub fn cassette_selected(&self) -> bool {
        self.control & 0x40 != 0
    }

    fn baud(code: u8) -> u32 {
        match code & 0x07 {
            0 => 19200,
            1 => 1200,
            2 => 4800,
            3 => 150,
            4 => 9600,
            5 => 300,
            6 => 2400,
            7 => 75,
            _ => unreachable!(),
        }
    }

    pub fn rx_baud(&self) -> u32 {
        Self::baud(self.control >> 3)
    }

    pub fn tx_baud(&self) -> u32 {
        Self::baud(self.control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_reset_via_control_register() {
        let mut a = Acia6850::new();
        // Pre-load some state.
        a.write(1, 0xAA);
        a.write(0, 0x03); // master reset
        assert_eq!(a.tdr, 0xAA); // TDR not cleared by master reset
        let status = a.read(0);
        assert!(status & 0x02 != 0); // TDRE
        assert!(status & 0x01 == 0); // RDRF clear
    }

    #[test]
    fn receive_then_read_clears_rdrf() {
        let mut a = Acia6850::new();
        a.write(0, 0x96); // 8N1, RX irq enable
        a.receive(0x5A);
        assert!(a.read(0) & 0x01 != 0);
        assert_eq!(a.read(1), 0x5A);
        assert!(a.read(0) & 0x01 == 0);
    }

    #[test]
    fn double_receive_sets_overrun() {
        let mut a = Acia6850::new();
        a.write(0, 0x96);
        a.receive(0x11);
        a.receive(0x22);
        assert!(a.read(0) & 0x20 != 0);
    }

    #[test]
    fn serial_ula_baud_decode() {
        let mut ula = SerialUla::new();
        ula.write(0b0000_0000); // RX 19200 TX 19200
        assert_eq!(ula.rx_baud(), 19200);
        assert_eq!(ula.tx_baud(), 19200);
        ula.write(0b0010_0101);
        // bits 5:3 = 100 → 9600 (table from BBC AUG), bits 2:0 = 101 → 300
        assert_eq!(ula.rx_baud(), 9600);
        assert_eq!(ula.tx_baud(), 300);
        assert!(!ula.motor_on());
        assert!(!ula.cassette_selected());
        ula.write(0xC0);
        assert!(ula.motor_on());
        assert!(ula.cassette_selected());
    }
}
