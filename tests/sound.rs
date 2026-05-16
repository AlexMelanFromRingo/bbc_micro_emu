//! Verifies the SN76489 synth + WAV dump path.

use bbc_micro_emu::sn76489::Sn76489;

#[test]
fn sn76489_pure_tone_generates_alternating_pcm() {
    let mut s = Sn76489::new();
    // Channel 0: period $100 (mid-range), volume = 0 (loud).
    s.write(0x80); // latch ch0 freq, low nibble = 0
    s.write(0x10); // continuation, high 6 bits = $10 → period = $100
    s.write(0x90); // latch ch0 volume = 0
    let pcm = s.synthesize(44_100, 2048);

    let positive = pcm.iter().filter(|&&v| v > 100).count();
    let negative = pcm.iter().filter(|&&v| v < -100).count();
    assert!(
        positive > 100 && negative > 100,
        "expected square-wave PCM with both halves loud — got {positive} positive, {negative} negative",
    );
}

#[test]
fn sn76489_wav_dump_round_trips_header() {
    let mut s = Sn76489::new();
    s.write(0x80); // ch0 freq low = 0
    s.write(0x10); // ch0 freq high = $10
    s.write(0x90); // ch0 volume = 0

    let path = std::env::temp_dir().join("bbc_micro_emu_test_tone.wav");
    s.dump_wav(&path, 44_100, 0.25).unwrap();

    // Validate the WAV header.
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"RIFF"), "no RIFF magic");
    assert_eq!(&bytes[8..12], b"WAVE", "no WAVE magic");
    assert_eq!(&bytes[12..16], b"fmt ", "no fmt chunk");
    assert_eq!(&bytes[20..22], &1u16.to_le_bytes(), "not PCM");
    assert_eq!(&bytes[22..24], &1u16.to_le_bytes(), "not mono");
    assert_eq!(
        &bytes[24..28],
        &44_100u32.to_le_bytes(),
        "wrong sample rate"
    );
    // Data length: 44100 * 0.25 = 11025 samples × 2 bytes = 22050.
    let data_size = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
    assert_eq!(data_size, 22_050);
    assert_eq!(bytes.len(), 44 + 22_050);
    std::fs::remove_file(&path).ok();
}

#[test]
fn sn76489_is_audible_only_after_volume_set() {
    let mut s = Sn76489::new();
    assert!(!s.is_audible(), "should be silent at power-on");
    s.write(0x80);
    s.write(0x10);
    assert!(!s.is_audible(), "still silent without volume command");
    s.write(0x90);
    assert!(
        s.is_audible(),
        "should be audible after volume + period set"
    );
}
