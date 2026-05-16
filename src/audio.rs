//! Real-time audio output via cpal. Owns a background cpal stream that
//! pulls samples from the shared [`Sn76489`] on each device callback.
//!
//! Wiring (in `display.rs`):
//! ```ignore
//! let machine = Machine::new(cfg)?;
//! let audio = AudioOutput::spawn(machine.bus.hardware.sound_handle());
//! // …run the winit event loop normally; audio runs in its own thread.
//! ```
//!
//! When the `audio` feature is disabled, this module exposes a no-op
//! [`AudioOutput`] so `display.rs` doesn't need cfg-bracketing.

#[cfg(feature = "audio")]
mod real {
    use std::sync::{Arc, Mutex};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use crate::sn76489::Sn76489;

    /// Owns the cpal stream + chip handle. Drop to stop playback.
    pub struct AudioOutput {
        _stream: cpal::Stream,
    }

    impl AudioOutput {
        /// Bring up the default output device and start streaming PCM
        /// generated from `chip`. Returns `None` if the host has no
        /// default device (CI, sandboxed runtimes), so callers can fall
        /// back gracefully.
        pub fn spawn(chip: Arc<Mutex<Sn76489>>) -> Option<Self> {
            let host = cpal::default_host();
            let device = host.default_output_device()?;
            let config = device.default_output_config().ok()?;
            let sample_rate = config.sample_rate().0;
            let channels = config.channels() as usize;
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => Self::build_stream::<f32>(
                    &device,
                    &config.into(),
                    chip.clone(),
                    sample_rate,
                    channels,
                    |s| s as f32 / 32_767.0,
                )?,
                cpal::SampleFormat::I16 => Self::build_stream::<i16>(
                    &device,
                    &config.into(),
                    chip.clone(),
                    sample_rate,
                    channels,
                    |s| s,
                )?,
                cpal::SampleFormat::U16 => Self::build_stream::<u16>(
                    &device,
                    &config.into(),
                    chip.clone(),
                    sample_rate,
                    channels,
                    |s| (s as i32 + 32_768) as u16,
                )?,
                _ => return None,
            };
            stream.play().ok()?;
            Some(Self { _stream: stream })
        }

        fn build_stream<T>(
            device: &cpal::Device,
            config: &cpal::StreamConfig,
            chip: Arc<Mutex<Sn76489>>,
            sample_rate: u32,
            channels: usize,
            convert: impl Fn(i16) -> T + Send + 'static,
        ) -> Option<cpal::Stream>
        where
            T: cpal::SizedSample + Send + 'static,
        {
            device
                .build_output_stream(
                    config,
                    move |buf: &mut [T], _| {
                        let frames = buf.len() / channels.max(1);
                        let pcm = chip
                            .lock()
                            .ok()
                            .map(|mut c| c.synthesize(sample_rate, frames))
                            .unwrap_or_else(|| vec![0i16; frames]);
                        for (frame_idx, sample) in pcm.into_iter().enumerate() {
                            let v = convert(sample);
                            for ch in 0..channels {
                                buf[frame_idx * channels + ch] = v;
                            }
                        }
                    },
                    |err| eprintln!("audio stream error: {err}"),
                    None,
                )
                .ok()
        }
    }
}

#[cfg(feature = "audio")]
pub use real::AudioOutput;

#[cfg(not(feature = "audio"))]
mod stub {
    use std::sync::{Arc, Mutex};

    use crate::sn76489::Sn76489;

    /// No-op audio output for builds compiled without `--features audio`.
    pub struct AudioOutput;

    impl AudioOutput {
        pub fn spawn(_chip: Arc<Mutex<Sn76489>>) -> Option<Self> {
            None
        }
    }
}

#[cfg(not(feature = "audio"))]
pub use stub::AudioOutput;
