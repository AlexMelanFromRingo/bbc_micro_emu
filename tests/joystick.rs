//! Joystick wiring (Machine::set_joystick_*).

use bbc_micro_emu::{Machine, MachineConfig, MemoryConfig};

fn fresh() -> Machine {
    Machine::new(MachineConfig {
        memory: MemoryConfig::default(),
    })
    .unwrap()
}

#[test]
fn joystick_axis_value_round_trips_through_adc() {
    let mut m = fresh();
    m.set_joystick_axis(0, -16384);
    m.set_joystick_axis(1, 16383);
    // Trigger conversion on channel 0, wait for it.
    m.bus.hardware.adc.write(0, 0x00);
    m.bus.hardware.adc.tick(8_001);
    let hi0 = m.bus.hardware.adc.read(1);
    // -16384 → unsigned 16384 = $4000 → top byte $40.
    assert_eq!(hi0, 0x40, "channel 0 hi byte");

    m.bus.hardware.adc.write(0, 0x01); // channel 1
    m.bus.hardware.adc.tick(8_001);
    let hi1 = m.bus.hardware.adc.read(1);
    // 16383 → unsigned 49151 = $BFFF → top byte $BF.
    assert_eq!(hi1, 0xBF, "channel 1 hi byte");
}

#[test]
fn joystick_button_drives_via_pb_low_when_pressed() {
    let mut m = fresh();
    // Default IRB is whatever the VIA powered up with; explicitly set bits 4,5
    // high so we have a known release-state baseline.
    m.bus.hardware.system_via.via.irb |= 0x30;
    let baseline = m.bus.hardware.system_via.via.irb & 0x30;
    assert_eq!(baseline, 0x30, "both buttons high (released)");

    m.set_joystick_button(0, true);
    assert_eq!(m.bus.hardware.system_via.via.irb & 0x10, 0x00, "fire 1 low");
    assert_eq!(
        m.bus.hardware.system_via.via.irb & 0x20,
        0x20,
        "fire 2 high"
    );

    m.set_joystick_button(0, false);
    m.set_joystick_button(1, true);
    assert_eq!(m.bus.hardware.system_via.via.irb & 0x20, 0x00, "fire 2 low");
    assert_eq!(
        m.bus.hardware.system_via.via.irb & 0x10,
        0x10,
        "fire 1 back high"
    );
}
