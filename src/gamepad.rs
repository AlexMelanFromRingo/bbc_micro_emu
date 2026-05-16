//! Bridge from a host gamepad (via `gilrs`) to `Machine::set_joystick_*`.
//!
//! Convention: gilrs' Active gamepad → BBC joystick 1.
//!   - Left-stick X axis  → channel 0
//!   - Left-stick Y axis  → channel 1
//!   - South button (A)   → fire 1
//!   - East button (B)    → fire 2
//!
//! Build with `--features gamepad` (default on) to enable. Without the
//! feature this module exposes a no-op stub.

#[cfg(feature = "gamepad")]
mod real {
    use crate::machine::Machine;
    use gilrs::{Axis, Button, EventType, Gilrs};

    pub struct GamepadBridge {
        gilrs: Option<Gilrs>,
    }

    impl GamepadBridge {
        pub fn new() -> Self {
            let gilrs = match Gilrs::new() {
                Ok(g) => Some(g),
                Err(e) => {
                    eprintln!("gamepad: gilrs init failed ({e:?}); disabled");
                    None
                }
            };
            Self { gilrs }
        }

        /// Drain pending gilrs events and forward to the machine's joystick
        /// API. Call once per redraw.
        pub fn pump(&mut self, machine: &mut Machine) {
            let Some(gilrs) = self.gilrs.as_mut() else {
                return;
            };
            while let Some(ev) = gilrs.next_event() {
                match ev.event {
                    EventType::AxisChanged(axis, value, _) => {
                        // gilrs reports axis as f32 in [-1.0, 1.0]; BBC ADC
                        // expects signed i16 in [-32768, 32767]. Flip Y so
                        // pushing up gives "negative" = "up" by Elite's
                        // convention.
                        let v = (value * 32_767.0).clamp(-32_767.0, 32_767.0) as i16;
                        match axis {
                            Axis::LeftStickX => machine.set_joystick_axis(0, v),
                            Axis::LeftStickY => machine.set_joystick_axis(1, -v),
                            _ => {}
                        }
                    }
                    EventType::ButtonPressed(b, _) => match b {
                        Button::South => machine.set_joystick_button(0, true),
                        Button::East => machine.set_joystick_button(1, true),
                        _ => {}
                    },
                    EventType::ButtonReleased(b, _) => match b {
                        Button::South => machine.set_joystick_button(0, false),
                        Button::East => machine.set_joystick_button(1, false),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }
}

#[cfg(feature = "gamepad")]
pub use real::GamepadBridge;

#[cfg(not(feature = "gamepad"))]
mod stub {
    use crate::machine::Machine;

    pub struct GamepadBridge;

    impl GamepadBridge {
        pub fn new() -> Self {
            Self
        }
        pub fn pump(&mut self, _m: &mut Machine) {}
    }
}

#[cfg(not(feature = "gamepad"))]
pub use stub::GamepadBridge;

impl Default for GamepadBridge {
    fn default() -> Self {
        Self::new()
    }
}
