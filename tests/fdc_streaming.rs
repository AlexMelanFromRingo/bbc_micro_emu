//! End-to-end FDC streaming test. Builds a tiny synthetic SSD with a single
//! file ("HELLO"), issues a Read Data command, and verifies the NMI stream
//! delivers the sector bytes back through the data register exactly as the
//! real 8271 → 6502 NMI handler would see them.

use bbc_micro_emu::fdc8271::{Fdc8271, SECTOR_SIZE, SECTORS_PER_TRACK, SSD_SIZE, TRACKS_PER_DISK};

/// Acorn DFS catalogue layout:
/// - Sector 0: disk title (8 chars), 31 file-name entries (8 bytes each).
/// - Sector 1: title continuation (4 chars), then per-entry metadata.
///
/// We populate ONE file called "HELLO" with 256 bytes of payload starting at
/// disc sector 2. Load/exec addresses are dummy.
fn build_minimal_ssd() -> Vec<u8> {
    let mut img = vec![0u8; SSD_SIZE];

    // Sector 0 — disk title + first file name.
    let title = b"TESTDISK";
    img[..title.len()].copy_from_slice(title);
    // First file entry at offset 8: name "HELLO" padded, dir letter '$'.
    let name = b"HELLO  ";
    img[8..15].copy_from_slice(name);
    img[15] = b'$';

    // Sector 1 — title continuation, cycle, count*8, sector count.
    let s1 = SECTOR_SIZE;
    img[s1..s1 + 4].copy_from_slice(b"\0\0\0\0"); // title continuation (unused)
    img[s1 + 4] = 0x00; // cycle (BCD)
    img[s1 + 5] = 8; // number of files × 8 (1 file)
    img[s1 + 6] = 0x00; // bits 8-9 of total sectors, sector size, opt boot
    img[s1 + 7] = (TRACKS_PER_DISK * SECTORS_PER_TRACK) as u8; // 800 & 0xFF
    // File entry metadata at offset 8 of sector 1:
    // bytes 0-1: load addr low (here $0000)
    // bytes 2-3: exec addr low (here $0000)
    // bytes 4-5: length low (256 bytes = $0100)
    // byte 6:   bits 0-1 = start sector high; bits 2-3 = length high;
    //           bits 4-5 = exec high; bits 6-7 = load high
    // byte 7:   start sector low
    img[s1 + 8] = 0x00; // load lo
    img[s1 + 8 + 1] = 0x00;
    img[s1 + 8 + 2] = 0x00; // exec lo
    img[s1 + 8 + 3] = 0x00;
    img[s1 + 8 + 4] = 0x00; // length lo (256)
    img[s1 + 8 + 5] = 0x01;
    img[s1 + 8 + 6] = 0x00; // packed high bits all zero
    img[s1 + 8 + 7] = 0x02; // start sector = 2

    // File data at track 0, sector 2 — fill with recognisable pattern.
    let data_off = 2 * SECTOR_SIZE;
    for i in 0..SECTOR_SIZE {
        img[data_off + i] = b'A' + ((i as u8) & 0x1F);
    }

    img
}

#[test]
fn read_drive_status_reports_drive_loaded() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, build_minimal_ssd()).unwrap();
    // Command 0x2C = Read Drive Status, no parameters.
    fdc.write(0, 0x2C);
    let status = fdc.read(0);
    assert!(
        status & 0x10 != 0,
        "RESULT_FULL must be set after Read Drive Status, got ${status:02X}"
    );
    let result = fdc.read(1);
    assert!(result & 0x80 != 0, "drive 0 'ready' bit should be set");
    assert!(result & 0x04 != 0, "drive 0 'present' bit should be set");
    assert!(result & 0x02 != 0, "track 0 detect bit should be set");
}

#[test]
fn read_data_streams_full_sector_via_nmi() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, build_minimal_ssd()).unwrap();

    // Specify command (0x35) — required init before reads on real DFS.
    fdc.write(0, 0x35);
    for byte in [0x0D, 0x0A, 0x0A, 0x09] {
        fdc.write(1, byte);
    }
    // Tick enough for Specify to complete; drain any NMI edge.
    fdc.tick(1000);
    let _ = fdc.poll_nmi_edge();
    let _ = fdc.read(1); // ack result

    // Read Data multi-sector (0x13): track=0, sector=2, count=1.
    fdc.write(0, 0x13);
    fdc.write(1, 0x00);
    fdc.write(1, 0x02);
    fdc.write(1, 0x01);

    // Drive the FDC like the real CPU NMI handler would: each NMI edge,
    // read the data register. Continue until command completes.
    let mut bytes_read = Vec::new();
    let mut watchdog = 0u32;
    loop {
        fdc.tick(64); // ~64 CPU cycles per byte at 2 MHz / 256-byte sector ≈ realistic spacing
        if fdc.poll_nmi_edge() {
            let s = fdc.read(0);
            if s & 0x04 != 0 {
                // NDDR — data byte ready
                bytes_read.push(fdc.read(4));
            }
            if s & 0x10 != 0 && bytes_read.len() == SECTOR_SIZE {
                // Final completion; just ack.
                let _ = fdc.read(1);
                break;
            }
        }
        watchdog += 1;
        if watchdog > 100_000 {
            panic!(
                "FDC did not finish sector read after {watchdog} ticks (got {} bytes)",
                bytes_read.len()
            );
        }
    }

    assert_eq!(bytes_read.len(), SECTOR_SIZE);
    for (i, &got) in bytes_read.iter().enumerate() {
        let expected = b'A' + ((i as u8) & 0x1F);
        assert_eq!(
            got, expected,
            "byte {i} mismatch: got ${got:02X} expected ${expected:02X}"
        );
    }
}

#[test]
fn write_then_read_round_trips_a_sector() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, vec![0u8; SSD_SIZE]).unwrap();

    // Write Data (0x0B): track=5, sector=3, count=1.
    fdc.write(0, 0x0B);
    fdc.write(1, 0x05);
    fdc.write(1, 0x03);
    fdc.write(1, 0x01);

    // Feed 256 bytes — each NMI consumes one byte from the data register.
    let payload: Vec<u8> = (0..SECTOR_SIZE).map(|i| (i ^ 0x55) as u8).collect();
    let mut idx = 0usize;
    let mut watchdog = 0u32;
    while idx <= SECTOR_SIZE {
        fdc.tick(64);
        if fdc.poll_nmi_edge() {
            let s = fdc.read(0);
            if s & 0x04 != 0 && idx < SECTOR_SIZE {
                fdc.write(4, payload[idx]);
                idx += 1;
            } else if s & 0x10 != 0 {
                let _ = fdc.read(1);
                break;
            }
        }
        watchdog += 1;
        if watchdog > 100_000 {
            panic!("write timed out after sending {idx} bytes");
        }
    }

    // Read it back via Read Data.
    fdc.write(0, 0x13);
    fdc.write(1, 0x05);
    fdc.write(1, 0x03);
    fdc.write(1, 0x01);
    let mut readback = Vec::new();
    let mut watchdog = 0u32;
    loop {
        fdc.tick(64);
        if fdc.poll_nmi_edge() {
            let s = fdc.read(0);
            if s & 0x04 != 0 {
                readback.push(fdc.read(4));
            }
            if s & 0x10 != 0 && readback.len() == SECTOR_SIZE {
                let _ = fdc.read(1);
                break;
            }
        }
        watchdog += 1;
        if watchdog > 100_000 {
            panic!("read timed out after {watchdog} ticks");
        }
    }
    assert_eq!(readback, payload, "round-trip data did not match");
}
