//! winit + softbuffer window for the BBC Micro display.
//!
//! Targets a 640×512 framebuffer (4:3 PAL aspect after scaling). The event loop
//! advances the machine by one frame per redraw request.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::machine::Machine;
use crate::renderer::{Framebuffer, SCREEN_H, SCREEN_W};
use crate::system_via::BbcKey;

fn map_keycode(code: KeyCode) -> Option<BbcKey> {
    use BbcKey::*;
    Some(match code {
        KeyCode::Escape => Escape,
        KeyCode::Tab => Tab,
        KeyCode::Enter => Return,
        KeyCode::Space => Space,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Shift,
        KeyCode::ControlLeft | KeyCode::ControlRight => Ctrl,
        KeyCode::CapsLock => CapsLock,
        KeyCode::ArrowUp => Up,
        KeyCode::ArrowDown => Down,
        KeyCode::ArrowLeft => Left,
        KeyCode::ArrowRight => Right,
        KeyCode::KeyA => KeyA,
        KeyCode::KeyB => KeyB,
        KeyCode::KeyC => KeyC,
        KeyCode::KeyD => KeyD,
        KeyCode::KeyE => KeyE,
        KeyCode::KeyF => KeyF,
        KeyCode::KeyG => KeyG,
        KeyCode::KeyH => KeyH,
        KeyCode::KeyI => KeyI,
        KeyCode::KeyJ => KeyJ,
        KeyCode::KeyK => KeyK,
        KeyCode::KeyL => KeyL,
        KeyCode::KeyM => KeyM,
        KeyCode::KeyN => KeyN,
        KeyCode::KeyO => KeyO,
        KeyCode::KeyP => KeyP,
        KeyCode::KeyQ => KeyQ,
        KeyCode::KeyR => KeyR,
        KeyCode::KeyS => KeyS,
        KeyCode::KeyT => KeyT,
        KeyCode::KeyU => KeyU,
        KeyCode::KeyV => KeyV,
        KeyCode::KeyW => KeyW,
        KeyCode::KeyX => KeyX,
        KeyCode::KeyY => KeyY,
        KeyCode::KeyZ => KeyZ,
        KeyCode::Digit0 => K0,
        KeyCode::Digit1 => K1,
        KeyCode::Digit2 => K2,
        KeyCode::Digit3 => K3,
        KeyCode::Digit4 => K4,
        KeyCode::Digit5 => K5,
        KeyCode::Digit6 => K6,
        KeyCode::Digit7 => K7,
        KeyCode::Digit8 => K8,
        KeyCode::Digit9 => K9,
        KeyCode::Comma => Comma,
        KeyCode::Period => Period,
        KeyCode::Slash => Slash,
        KeyCode::Semicolon => Semicolon,
        KeyCode::Quote => Colon,
        KeyCode::Minus => Minus,
        KeyCode::Equal => Equals,
        _ => return None,
    })
}

pub struct DisplayApp {
    machine: Machine,
    fb: Framebuffer,
    last_frame: Instant,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    context: Option<Context<Rc<Window>>>,
    title_prefix: String,
    /// Keeps the cpal stream alive for the lifetime of the window.
    /// Dropping it (or initialisation failure) silently disables audio.
    _audio: Option<crate::audio::AudioOutput>,
    fps_accum: u32,
    fps_window_start: Instant,
}

impl DisplayApp {
    pub fn new(machine: Machine, title: impl Into<String>) -> Self {
        let audio = crate::audio::AudioOutput::spawn(machine.bus.hardware.sound_handle());
        if audio.is_none() {
            eprintln!(
                "audio: no default output device (or built with --no-default-features); silent"
            );
        }
        Self {
            machine,
            fb: Framebuffer::new(),
            last_frame: Instant::now(),
            window: None,
            surface: None,
            context: None,
            title_prefix: title.into(),
            _audio: audio,
            fps_accum: 0,
            fps_window_start: Instant::now(),
        }
    }

    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(&mut self)?;
        Ok(())
    }
}

impl ApplicationHandler for DisplayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = WindowAttributes::default()
            .with_title(format!("{} — BBC Micro", self.title_prefix))
            .with_inner_size(winit::dpi::LogicalSize::new(
                SCREEN_W as f64,
                SCREEN_H as f64,
            ));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(err) => {
                eprintln!("failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };
        let context = Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("softbuffer surface");
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        state,
                        text,
                        logical_key,
                        ..
                    },
                ..
            } => {
                if let PhysicalKey::Code(code) = physical_key
                    && let Some(key) = map_keycode(code)
                {
                    self.machine
                        .bus
                        .hardware
                        .system_via
                        .set_key(key, state == ElementState::Pressed);
                }
                // Also feed the character into the OSRDCH passthrough queue so
                // BASIC's command line picks it up reliably regardless of
                // System VIA scan timing.
                if state == ElementState::Pressed {
                    if let Some(text) = text.as_deref() {
                        for b in text.bytes() {
                            self.machine.type_byte(if b == b'\n' { 0x0D } else { b });
                        }
                    } else if let winit::keyboard::Key::Named(named) = logical_key {
                        use winit::keyboard::NamedKey;
                        match named {
                            NamedKey::Enter => self.machine.type_byte(0x0D),
                            NamedKey::Backspace => self.machine.type_byte(0x7F),
                            NamedKey::Escape => self.machine.type_byte(0x1B),
                            _ => {}
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                // Real-time pace: at 2 MHz one PAL frame = 40 000 cycles =
                // 20 ms. Compute how many full frames of CPU time have
                // elapsed in wall-time since the last redraw and run
                // exactly that much, capped at 4 frames so a stalled host
                // can't make us spiral. If we're caught up, we'll naturally
                // sleep at the bottom of the handler.
                let frame_micros = 20_000u128;
                let elapsed = self.last_frame.elapsed().as_micros();
                let mut frames_to_run = (elapsed / frame_micros).max(1) as u32;
                if frames_to_run > 4 {
                    frames_to_run = 4;
                }
                for _ in 0..frames_to_run {
                    if let Err(err) = self.machine.run_one_frame() {
                        eprintln!("machine error: {err}");
                        event_loop.exit();
                        return;
                    }
                }
                self.machine.render_into(&mut self.fb);
                // Update title with measured FPS.
                if let Some(win) = self.window.as_ref() {
                    self.fps_accum += 1;
                    if self.fps_accum >= 50 {
                        let secs = self.fps_window_start.elapsed().as_secs_f32();
                        let fps = self.fps_accum as f32 / secs.max(0.001);
                        win.set_title(&format!(
                            "{} — BBC Micro · {fps:.0} fps",
                            self.title_prefix
                        ));
                        self.fps_accum = 0;
                        self.fps_window_start = Instant::now();
                    }
                }

                if let (Some(surface), Some(window)) = (self.surface.as_mut(), self.window.as_ref())
                {
                    let size = window.inner_size();
                    let (w, h) = (size.width.max(1), size.height.max(1));
                    if let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) {
                        let _ = surface.resize(nw, nh);
                        if let Ok(mut buf) = surface.buffer_mut() {
                            // Scale source 640×512 to target w×h with simple
                            // nearest-neighbour. Most users will see exact 1:1.
                            let dst_w = w as usize;
                            let dst_h = h as usize;
                            for y in 0..dst_h {
                                let sy = y * SCREEN_H / dst_h;
                                let src_row = &self.fb.pixels[sy * SCREEN_W..(sy + 1) * SCREEN_W];
                                let dst_row = &mut buf[y * dst_w..(y + 1) * dst_w];
                                for (x, dst) in dst_row.iter_mut().enumerate() {
                                    let sx = x * SCREEN_W / dst_w;
                                    *dst = src_row[sx];
                                }
                            }
                            let _ = buf.present();
                        }
                    }
                }

                // Throttle to ~50 Hz.
                let target = Duration::from_micros(20_000);
                let elapsed = self.last_frame.elapsed();
                if elapsed < target {
                    std::thread::sleep(target - elapsed);
                }
                self.last_frame = Instant::now();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
