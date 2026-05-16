//! Double-sided disc image (DSD) verification.
//!
//! Builds a synthetic 400 KiB interleaved DSD where every sector is filled
//! with a recognisable byte that encodes (track, side, sector). Reads each
//! sector back through the 8271 streaming pipeline and checks the bytes.

use bbc_micro_emu::fdc8271::{DSD_SIZE, Fdc8271, SECTOR_SIZE, SECTORS_PER_TRACK, TRACKS_PER_DISK};

const TRACK_PAIR: usize = SECTORS_PER_TRACK * SECTOR_SIZE;

fn build_pattern_dsd() -> Vec<u8> {
    let mut img = vec![0u8; DSD_SIZE];
    for t in 0..TRACKS_PER_DISK {
        for side in 0..2 {
            for s in 0..SECTORS_PER_TRACK {
                // Interleaved layout: each track is 2 × 10 sectors back-to-back.
                let track_base = t * TRACK_PAIR * 2 + side * TRACK_PAIR;
                let off = track_base + s * SECTOR_SIZE;
                let byte = pattern_byte(t, side, s);
                for i in 0..SECTOR_SIZE {
                    img[off + i] = byte ^ (i as u8);
                }
            }
        }
    }
    img
}

fn pattern_byte(track: usize, side: usize, sector: usize) -> u8 {
    ((track * 17 + side * 53 + sector * 7) & 0xFF) as u8
}

fn read_sector(fdc: &mut Fdc8271, track: u8, sector: u8) -> Vec<u8> {
    fdc.write(0, 0x13); // Read Data multi-sector
    fdc.write(1, track);
    fdc.write(1, sector);
    fdc.write(1, 1); // count = 1
    let mut bytes = Vec::with_capacity(SECTOR_SIZE);
    let mut watchdog = 0u32;
    fdc.tick(8500); // initial head-settle + sector search
    loop {
        if fdc.poll_nmi_edge() {
            let status = fdc.read(0);
            if status & 0x04 != 0 {
                bytes.push(fdc.read(4));
            }
            if status & 0x10 != 0 && bytes.len() == SECTOR_SIZE {
                let _ = fdc.read(1);
                break;
            }
        }
        fdc.tick(150);
        watchdog += 1;
        if watchdog > 100_000 {
            panic!("read_sector timed out (got {} bytes)", bytes.len());
        }
    }
    bytes
}

#[test]
fn dsd_read_side_0_returns_side_0_pattern() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, build_pattern_dsd()).unwrap();
    // mmioDriveOut $23 = select_0 | loadHead; side bit 0x20 = 0 = side 0.
    fdc.write(0, 0x3A);
    fdc.write(1, 0x23);
    fdc.write(1, 0x48);
    fdc.tick(2_000);

    let bytes = read_sector(&mut fdc, 5, 3);
    let expected = pattern_byte(5, 0, 3);
    for (i, &b) in bytes.iter().enumerate() {
        assert_eq!(b, expected ^ (i as u8), "side 0 track 5 sector 3 byte {i}");
    }
}

#[test]
fn dsd_read_side_1_returns_distinct_pattern() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, build_pattern_dsd()).unwrap();
    // mmioDriveOut $23 = select_0 | side | loadHead = $40 | $20 | $08 = $68.
    fdc.write(0, 0x3A);
    fdc.write(1, 0x23);
    fdc.write(1, 0x68);
    fdc.tick(2_000);

    let bytes = read_sector(&mut fdc, 5, 3);
    let expected_side1 = pattern_byte(5, 1, 3);
    let expected_side0 = pattern_byte(5, 0, 3);
    assert_ne!(expected_side1, expected_side0, "patterns must differ");
    for (i, &b) in bytes.iter().enumerate() {
        assert_eq!(
            b,
            expected_side1 ^ (i as u8),
            "side 1 track 5 sector 3 byte {i}"
        );
    }
}

#[test]
fn dsd_image_load_succeeds() {
    let mut fdc = Fdc8271::new();
    fdc.load_image(0, vec![0xCDu8; DSD_SIZE]).unwrap();
    assert!(fdc.has_disk());
}
