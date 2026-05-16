//! Round-trip the snapshot format.

use bbc_micro_emu::snapshot::{MAGIC, SNAPSHOT_LEN, VERSION, decode, encode};
use bbc_micro_emu::{Machine, MachineConfig, MemoryConfig};

fn fresh() -> Machine {
    Machine::new(MachineConfig {
        memory: MemoryConfig::default(),
    })
    .unwrap()
}

#[test]
fn snapshot_header_layout() {
    let m = fresh();
    let buf = encode(&m);
    assert_eq!(buf.len(), SNAPSHOT_LEN);
    assert_eq!(&buf[0..4], MAGIC);
    assert_eq!(buf[4], VERSION);
}

#[test]
fn snapshot_round_trip_preserves_cpu_and_ram() {
    let mut a = fresh();

    // Mutate something interesting in each subsystem.
    a.cpu.registers.a = 0x42;
    a.cpu.registers.x = 0x37;
    a.cpu.registers.y = 0x11;
    a.cpu.registers.pc = 0xABCD;
    a.cpu.cycles = 1_234_567;
    a.bus.memory.ram_mut()[0x1000] = 0xAB;
    a.bus.memory.ram_mut()[0x7FFF] = 0xCD;
    a.bus.hardware.crtc.write(0, 12);
    a.bus.hardware.crtc.write(1, 0x40);
    a.bus.hardware.video_ula.control = 0xC8;
    a.bus.hardware.video_ula.palette[5] = 0x09;
    a.bus.hardware.system_via.ic32 = 0x55;

    let snap = encode(&a);
    let mut b = fresh();
    decode(&mut b, &snap).expect("decode");

    assert_eq!(b.cpu.registers.a, 0x42);
    assert_eq!(b.cpu.registers.x, 0x37);
    assert_eq!(b.cpu.registers.y, 0x11);
    assert_eq!(b.cpu.registers.pc, 0xABCD);
    assert_eq!(b.cpu.cycles, 1_234_567);
    assert_eq!(b.bus.memory.ram()[0x1000], 0xAB);
    assert_eq!(b.bus.memory.ram()[0x7FFF], 0xCD);
    assert_eq!(b.bus.hardware.crtc.reg(12), 0x40);
    assert_eq!(b.bus.hardware.video_ula.control, 0xC8);
    assert_eq!(b.bus.hardware.video_ula.palette[5], 0x09);
    assert_eq!(b.bus.hardware.system_via.ic32, 0x55);
}

#[test]
fn snapshot_rejects_bad_magic_and_size() {
    let mut m = fresh();
    let mut garbage = vec![0u8; SNAPSHOT_LEN];
    garbage[0..4].copy_from_slice(b"XXXX");
    assert!(decode(&mut m, &garbage).is_err());

    let too_short = vec![0u8; SNAPSHOT_LEN - 1];
    assert!(decode(&mut m, &too_short).is_err());
}
