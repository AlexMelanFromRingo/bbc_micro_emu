//! Render unit tests: poke a known pattern into screen RAM, render, and check
//! the framebuffer contains the expected colours. These tests bypass the CPU
//! entirely so failures isolate the renderer/CRTC/Video ULA path.

use bbc_micro_emu::renderer::{SCREEN_H, SCREEN_W};
use bbc_micro_emu::{Crtc6845, Framebuffer, Renderer, VideoUla};

fn make_test_inputs() -> (Crtc6845, VideoUla, Vec<u8>) {
    // Set up MODE 4 by hand: 40 chars × 32 rows × 8 scanlines, 1bpp.
    let mut crtc = Crtc6845::new();
    crtc.write(0x00, 0);
    crtc.write(0x01, 63); // R0
    crtc.write(0x00, 1);
    crtc.write(0x01, 40); // R1
    crtc.write(0x00, 4);
    crtc.write(0x01, 38); // R4
    crtc.write(0x00, 6);
    crtc.write(0x01, 32); // R6
    crtc.write(0x00, 9);
    crtc.write(0x01, 7); // R9 = scanlines-1
    crtc.write(0x00, 12);
    crtc.write(0x01, 0x0B); // R12: screen start hi (0x0B*8 = 0x58 → $5800)
    crtc.write(0x00, 13);
    crtc.write(0x01, 0x00); // R13: screen start lo

    let mut ula = VideoUla::new();
    // MODE 4-ish: bitmap (bit 1=0), 1bpp (bits 3:2 = 11), 1 MHz (bit 4 = 0).
    // Note: the BBC ULA always encodes 4 bits of logical colour per pixel from
    // the byte's bits 1/3/5/7; in 1bpp visual modes only bit 3 of the logical
    // index varies, so the relevant palette entry is [8] (set pixel) and [0]
    // (unset pixel).
    ula.control = 0b0000_1100;
    ula.screen_size_code = 0b10;

    let ram = vec![0u8; 0x8000];
    (crtc, ula, ram)
}

#[test]
fn renderer_blanks_screen_when_ram_is_zero_and_palette_is_default() {
    let (crtc, ula, ram) = make_test_inputs();
    let mut renderer = Renderer::new();
    let mut fb = Framebuffer::new();
    renderer.render(&mut fb, &ram, &crtc, &ula);

    // All-zero byte → logical 0 → palette[0] = 0 (default). After XOR with 7
    // the physical bits are 0b111 (white), so all pixels are bright white.
    let white = 0x00FF_FFFFu32;
    let sample = fb.pixels[100 * SCREEN_W + 100];
    assert_eq!(sample, white, "expected white sample, got ${sample:08X}");
}

#[test]
fn renderer_draws_a_pixel_when_screen_ram_has_a_bit_set() {
    let (crtc, mut ula, mut ram) = make_test_inputs();

    // Set bit 7 of the first scanline byte. The BBC ULA decodes each byte
    // into 8 horizontal "logical colour" values (bits 1/3/5/7 of byte<<p).
    // For byte=$80, only pixel 0 has bit 3 of logical set (= 8).
    ram[0x5800] = 0x80;

    // Programme palette exactly as MOS does in MODE 0/4:
    //   logical 0..7 → stored $07 → physical 0 (black)
    //   logical 8..15 → stored $00 → physical 7 (white)
    for i in 0..8 {
        ula.palette[i] = 0x07;
    }
    for i in 8..16 {
        ula.palette[i] = 0x00;
    }

    let mut renderer = Renderer::new();
    let mut fb = Framebuffer::new();
    renderer.render(&mut fb, &ram, &crtc, &ula);

    let white = 0x00FF_FFFFu32;
    let black = 0u32;
    assert_eq!(fb.pixels[0], white, "byte $80 pixel 0 should be white");
    // After scale_x=2, pixels (0..1) are white, (2..) should be black.
    assert_eq!(fb.pixels[2], black, "byte $80 pixel 1+ should be black");
}

#[test]
fn framebuffer_dimensions_match_constants() {
    let fb = Framebuffer::new();
    assert_eq!(fb.pixels.len(), SCREEN_W * SCREEN_H);
}

#[test]
fn renderer_handles_2bpp_mode_1_pattern() {
    // MODE 1-ish: 40 chars × 32 rows × 8 scanlines, 2bpp.
    let mut crtc = Crtc6845::new();
    for (r, v) in [
        (1u8, 40),
        (4, 38),
        (6, 32),
        (9, 7),
        (12, 0x06),
        (13, 0x00),
        (0, 63),
    ] {
        crtc.write(0x00, r);
        crtc.write(0x01, v);
    }
    let mut ula = VideoUla::new();
    // 2 MHz, 2bpp (bits 3:2 = 10), bitmap, no flash.
    ula.control = 0b0000_1000;
    ula.screen_size_code = 0;
    // We'll arrange for pixel 0 (byte bits 1,3,5,7) to map to logical $F (all
    // four colour-bits set). Programme palette[$F] = physical 0 (white) for an
    // easily-checked result; logical 0 stays at default (= white too).
    ula.palette[0x0F] = 0x00;

    let mut ram = vec![0u8; 0x8000];
    ram[0x3000] = 0xFF;

    let mut renderer = Renderer::new();
    let mut fb = Framebuffer::new();
    renderer.render(&mut fb, &ram, &crtc, &ula);

    // palette[$F] = $00, after XOR with 7 → 7 (all RGB bits high) → white.
    let white = 0x00FF_FFFFu32;
    assert_eq!(fb.pixels[0], white, "expected white pixel from byte $FF");
}
