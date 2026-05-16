//! End-to-end demo: a hand-written 6502 program configures MODE 7 directly,
//! pokes text into teletext screen RAM ($7C00), and halts. This proves the
//! CPU → SHEILA → CRTC → Video ULA → renderer pipeline works without MOS.

use bbc_micro_emu::{Framebuffer, Machine, MachineConfig, MemoryConfig};

fn assemble_demo() -> Vec<u8> {
    // 16K MOS image. Reset vector → $C000. Program at $C000.
    let mut rom = vec![0xFFu8; 0x4000];

    // The teletext message we want to render at row 12, col 12 of MODE 7.
    let message = b"BBC EMULATOR";
    // Address $7C00 + 12 * 40 + 12 = $7DAC
    const TEXT_DEST: u16 = 0x7DAC;
    const MSG_TEMPLATE: u16 = 0xC100; // copied into MOS at offset $0100

    // Program: configure SHEILA, copy message, halt.
    // We use absolute addresses since we have no zero-page restrictions yet.
    let mut code: Vec<u8> = vec![];

    // sei
    code.push(0x78);
    // ldx #$06          ; CRTC R6 — vertical displayed = 25 (but for MODE 7 it's 25 rows × 10 scanlines/row, so use the basic write)
    // Hardcode minimum CRTC programming needed for our renderer to look at
    // 40×25 cells of teletext.
    // For MODE 7 the BBC hardware maps CRTC address $0000..$03FF into the
    // 1 KiB teletext page at $7C00. MOS programs R12=$28 R13=$00 which gives
    // an effective screen-RAM offset of zero. We match that here.
    let crtc_writes: [(u8, u8); 6] = [
        (1, 40), // R1 horizontal displayed = 40
        (6, 25), // R6 vertical displayed = 25
        (9, 18), // R9 scanlines per char - 1 = 18 (25 × 19 lines = 475 scanlines)
        (12, 0x28),
        (13, 0x00),
        (0, 63),
    ];

    for (reg, val) in crtc_writes {
        // STA $FE00 = address latch, STA $FE01 = data
        code.push(0xA9);
        code.push(reg); // LDA #reg
        code.push(0x8D);
        code.push(0x00);
        code.push(0xFE); // STA $FE00
        code.push(0xA9);
        code.push(val); // LDA #val
        code.push(0x8D);
        code.push(0x01);
        code.push(0xFE); // STA $FE01
    }

    // Video ULA control register: teletext bit (bit 1) set, 1 MHz (bit 4 = 0)
    code.push(0xA9);
    code.push(0x02);
    code.push(0x8D);
    code.push(0x20);
    code.push(0xFE); // STA $FE20

    // Copy 12 bytes from $C100 (message in ROM) to $7DAC (screen RAM).
    // LDX #0 ; loop: LDA $C100,X ; STA $7DAC,X ; INX ; CPX #12 ; BNE loop
    code.push(0xA2);
    code.push(0x00); // LDX #0
    code.push(0xBD);
    code.push((MSG_TEMPLATE & 0xFF) as u8);
    code.push((MSG_TEMPLATE >> 8) as u8); // LDA $C100,X
    code.push(0x9D);
    code.push((TEXT_DEST & 0xFF) as u8);
    code.push((TEXT_DEST >> 8) as u8); // STA $7DAC,X
    code.push(0xE8); // INX
    code.push(0xE0);
    code.push(message.len() as u8); // CPX #12
    code.push(0xD0);
    code.push(0xF5); // BNE loop (back to LDA, -11 bytes)

    // halt: JMP *
    let halt_addr = 0xC000u16 + code.len() as u16;
    code.push(0x4C);
    code.push((halt_addr & 0xFF) as u8);
    code.push((halt_addr >> 8) as u8);

    // Copy code to ROM[0..] (so it sits at $C000).
    rom[..code.len()].copy_from_slice(&code);
    // Copy message at offset $0100 (= $C100 in CPU space).
    rom[0x0100..0x0100 + message.len()].copy_from_slice(message);
    // Reset vector → $C000
    rom[0x3FFC] = 0x00;
    rom[0x3FFD] = 0xC0;
    rom
}

#[test]
fn mode7_pipeline_renders_user_program_output() {
    let mem = MemoryConfig {
        mos_rom_bytes: Some(assemble_demo()),
        ..MemoryConfig::default()
    };
    let mut machine = Machine::new(MachineConfig { memory: mem }).unwrap();
    // Run plenty of cycles for the program to finish.
    let report = machine.run_for_cycles(2_000, u64::MAX).unwrap();
    assert!(report.instructions > 20);

    // Render and check the message bytes ended up in screen RAM.
    let mut fb = Framebuffer::new();
    machine.render_into(&mut fb);

    // Direct memory check: the text "BBC EMULATOR" should sit at $7DAC.
    let ram_at = |addr: u16| machine.bus.memory.ram()[addr as usize];
    let written: Vec<u8> = (0..12).map(|i| ram_at(0x7DAC + i)).collect();
    assert_eq!(written, b"BBC EMULATOR".to_vec());

    // The framebuffer should now contain at least *some* non-black pixels in
    // the row 12 area (y around 240..260 in 640×512 teletext layout).
    let mut non_black = 0;
    for y in 240..260 {
        for x in 0..bbc_micro_emu::renderer::SCREEN_W {
            if fb.pixels[y * bbc_micro_emu::renderer::SCREEN_W + x] != 0 {
                non_black += 1;
            }
        }
    }
    assert!(
        non_black > 100,
        "expected text on screen but got {non_black} non-black pixels"
    );

    // Save a screenshot for visual inspection on test failure (and just as a
    // bonus when -- --nocapture is used).
    let _ = fb.save_ppm(std::path::Path::new("/tmp/mode7_demo.ppm"));
}
