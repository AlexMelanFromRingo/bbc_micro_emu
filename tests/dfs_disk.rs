//! End-to-end test: boot MOS + DFS, load a synthetic disk image, type `*CAT`,
//! and verify DFS reads the catalogue via the 8271 FDC's NMI streaming path.

use std::path::PathBuf;

use bbc_micro_emu::fdc8271::{SECTOR_SIZE, SECTORS_PER_TRACK, SSD_SIZE, TRACKS_PER_DISK};
use bbc_micro_emu::{Machine, MachineConfig, MemoryConfig};

fn build_disk_with_one_file() -> Vec<u8> {
    let mut img = vec![0u8; SSD_SIZE];

    // Sector 0: disc title (8 chars) + first file entry.
    img[..8].copy_from_slice(b"TESTDISK");
    img[8..15].copy_from_slice(b"HELLO  ");
    img[15] = b'$';

    // Sector 1: cycle / count / per-file metadata.
    let s1 = SECTOR_SIZE;
    img[s1..s1 + 4].copy_from_slice(b"\0\0\0\0"); // title continuation
    img[s1 + 4] = 0x10; // cycle (BCD)
    img[s1 + 5] = 8; // file count × 8
    // Byte $06 packs: bits 7-6 = boot option, bits 5-4 = reserved,
    // bits 3-2 = sector-size MSB, bits 1-0 = total-sector count high bits.
    // 800 sectors = $0320, so high bits = 0b11 (= 3).
    img[s1 + 6] = 0x03;
    img[s1 + 7] = ((TRACKS_PER_DISK * SECTORS_PER_TRACK) & 0xFF) as u8;
    img[s1 + 8] = 0x00; // load addr lo
    img[s1 + 8 + 1] = 0x19; // load addr hi → $1900
    img[s1 + 8 + 2] = 0x00; // exec addr lo
    img[s1 + 8 + 3] = 0x19;
    img[s1 + 8 + 4] = 0x00; // length lo (256)
    img[s1 + 8 + 5] = 0x01;
    img[s1 + 8 + 6] = 0x00;
    img[s1 + 8 + 7] = 0x02; // start sector

    // Sector 2: file payload.
    let payload = b"HELLO FROM DISK!\n";
    img[2 * SECTOR_SIZE..2 * SECTOR_SIZE + payload.len()].copy_from_slice(payload);
    img
}

#[test]
#[ignore = "needs roms/os120.rom + roms/basic2.rom + roms/dfs098.rom"]
fn dfs_cat_prints_filename() {
    let mos = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/os120.rom");
    let basic = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/basic2.rom");
    let dfs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms/dfs098.rom");
    if !dfs.exists() {
        panic!(
            "DFS ROM missing: {}\nRun scripts/fetch_roms.sh first.",
            dfs.display()
        );
    }

    let mut mem = MemoryConfig {
        mos_rom_path: Some(mos),
        initial_bank: 15,
        ..MemoryConfig::default()
    };
    mem.rom_banks[14] = Some(dfs);
    mem.rom_banks[15] = Some(basic);
    let mut machine = Machine::new(MachineConfig { memory: mem }).unwrap();
    machine
        .bus
        .hardware
        .fdc
        .load_image(0, build_disk_with_one_file())
        .unwrap();

    // Boot.
    machine.run_for_cycles(8_000_000, u64::MAX).unwrap();
    // *CAT to list files (issues Read Data via DFS service ROM).
    machine.type_string("*CAT\n");
    machine.run_for_cycles(40_000_000, u64::MAX).unwrap();
    eprintln!("FDC accesses: {}", machine.bus.hardware.access_summary());
    let ram = machine.bus.memory.ram();
    eprintln!("NMI handler at $0D00..$0D20: {:02X?}", &ram[0x0D00..0x0D20]);
    eprintln!("PC after: ${:04X}", machine.cpu.registers.pc);

    let ram = machine.bus.memory.ram();
    // The disk title and filename should now appear somewhere in MODE 7 RAM
    // (or in MODE 0/4 framebuffer area, depending on which display mode is
    // active). We grep across the entire visible-text RAM.
    // Dump screen rows separately for clarity.
    for row in 0..25 {
        let addr = 0x7C00 + row * 40;
        let line = String::from_utf8_lossy(&ram[addr..addr + 40]);
        eprintln!("row {row:2}: {line:?}");
    }
    let window: Vec<u8> = ram[0x7C00..0x8000].to_vec();
    let text = String::from_utf8_lossy(&window);
    assert!(
        text.contains("TESTDISK") || text.contains("HELLO"),
        "expected disc title or filename to appear in teletext page after *CAT"
    );
}
