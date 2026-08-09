//! The VGA display-memory seam: a guest reaches the Graphics Controller data
//! path through ordinary memory accesses.
//!
//! Round 1 built the plane decode and the Graphics Controller read/write data
//! path; round 2 gave the device a single guest-facing entry point. Until
//! `MachineBus` routed the whole `0xA0000`–`0xBFFFF` aperture to it, both were
//! host-callable only — a guest could reach the legacy `0xB8000` text buffer
//! and nothing else.
//!
//! The aperture is registered as a fixed range rather than a static sub-range
//! because the *claimed* sub-range moves at runtime with Miscellaneous Output
//! RAM Enable and Graphics Controller Miscellaneous Memory Map Select.
//!
//! Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems,
//! chapter 2: Figure 2-75 (Memory Map Select windows), Figure 2-74
//! (Miscellaneous), Figures 2-33 / 2-34 (Memory Mode addressing), Figure 2-29
//! (Map Mask), Figures 2-71 / 2-72 / 2-73 (Read Map Select, Graphics Mode,
//! Write Mode Definitions). See `docs/vga-r2-mmio-entry-point.md`.

use devices::{
    VgaText, VGA_GC_BIT_MASK, VGA_GC_DATA, VGA_GC_INDEX, VGA_GC_MEMORY_MAP_A0000_64K, VGA_GC_MISC,
    VGA_GC_MISC_GRAPHICS_MODE, VGA_GC_MISC_MEMORY_MAP_SHIFT, VGA_GC_MODE, VGA_GC_READ_MAP_SELECT,
    VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES, VGA_SEQ_MEMORY_MODE,
    VGA_SEQ_MEMORY_MODE_EXTENDED, VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE, VGA_WINDOW_A0000_BASE,
};
use machine_pc::Machine;

/// `MOV DX, imm16` / `MOV AL, imm8` / `OUT DX, AL`.
///
/// Only the 8-bit port forms are used: this build's primary opcode map has
/// `E4`/`E6`/`EC`/`EE` but not the `ED`/`EF` accumulator forms.
#[rustfmt::skip]
fn out_imm(port: u16, value: u8) -> Vec<u8> {
    let [lo, hi] = port.to_le_bytes();
    vec![
        0xBA, lo, hi,   // MOV DX, port
        0xB0, value,    // MOV AL, value
        0xEE,           // OUT DX, AL
    ]
}

/// Program an indexed VGA register pair (index port then data port).
fn out_indexed(index_port: u16, data_port: u16, index: u8, value: u8) -> Vec<u8> {
    let mut code = out_imm(index_port, index);
    code.extend(out_imm(data_port, value));
    code
}

fn write_gc(index: u8, value: u8) -> Vec<u8> {
    out_indexed(VGA_GC_INDEX, VGA_GC_DATA, index, value)
}

fn write_seq(index: u8, value: u8) -> Vec<u8> {
    out_indexed(VGA_SEQ_INDEX, VGA_SEQ_DATA, index, value)
}

/// Select the `0xA0000`–`0xAFFFF` graphics window with planar addressing.
fn select_planar_a0000_window() -> Vec<u8> {
    let mut code = write_gc(
        VGA_GC_MISC,
        VGA_GC_MISC_GRAPHICS_MODE | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
    );
    code.extend(write_seq(
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
    ));
    code
}

/// 64 KiB BIOS image whose reset vector far-jumps to `F000:0000`.
///
/// Spec: Intel SDM Vol. 3 §9.1.4.
fn bios_image_64k(code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];
    rom[..code.len()].copy_from_slice(code);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

fn run_guest(code: Vec<u8>) -> Machine {
    let rom = bios_image_64k(&code);
    let mut m = Machine::with_bios_rom(1024 * 1024, &rom).expect("map BIOS image");
    m.reset();
    m.run(4096).expect("guest runs to HLT");
    assert!(m.cpu.halted, "guest halted");
    m
}

/// Write mode 0 through the aperture: Map Mask decides which planes take the
/// byte, and Read Map Select decides which plane a read returns.
///
/// Spec: IBM Figure 2-73 (Write Mode 0), Figure 2-29 (Map Mask), Figure 2-71
/// (Read Map Select).
#[test]
fn guest_reaches_write_mode_0_and_map_mask_through_the_aperture() {
    let mut code = select_planar_a0000_window();
    // Only map 2 is write-enabled.
    code.extend(write_seq(VGA_SEQ_MAP_MASK, 0b0100));
    code.extend(write_gc(VGA_GC_BIT_MASK, 0xFF));
    // Write mode 0, read mode 0, host odd/even read off.
    code.extend(write_gc(VGA_GC_MODE, 0x00));
    #[rustfmt::skip]
    code.extend_from_slice(&[
        0xB8, 0x00, 0xA0,       // MOV AX, 0xA000
        0x8E, 0xC0,             // MOV ES, AX
        0x31, 0xFF,             // XOR DI, DI
        0xB0, b'G',             // MOV AL, 'G'
        0x26, 0x88, 0x05,       // MOV ES:[DI], AL
    ]);
    // Read map 2 back: the byte the CPU wrote.
    code.extend(write_gc(VGA_GC_READ_MAP_SELECT, 2));
    #[rustfmt::skip]
    code.extend_from_slice(&[
        0x26, 0x8A, 0x05,       // MOV AL, ES:[DI]
        0xBA, 0x02, 0x04,       // MOV DX, 0x0402
        0xEE,                   // OUT DX, AL
    ]);
    // Read map 1: Map Mask kept the write out, so it is still zero. Bias it
    // into printable range so the debug console text stays readable.
    code.extend(write_gc(VGA_GC_READ_MAP_SELECT, 1));
    #[rustfmt::skip]
    code.extend_from_slice(&[
        0x26, 0x8A, 0x05,       // MOV AL, ES:[DI]
        0x04, 0x40,             // ADD AL, 0x40
        0xBA, 0x02, 0x04,       // MOV DX, 0x0402
        0xEE,                   // OUT DX, AL
        0xF4,                   // HLT
    ]);

    let m = run_guest(code);

    assert_eq!(
        m.debug_text(),
        "G@",
        "map 2 holds the written byte and map 1 is untouched"
    );
    assert_eq!(m.vga.plane_byte(2, 0), Some(b'G'));
    assert_eq!(m.vga.plane_byte(0, 0), Some(0x00));
    assert_eq!(m.vga.plane_byte(1, 0), Some(0x00));
    assert_eq!(m.vga.plane_byte(3, 0), Some(0x00));
}

/// Write mode 2 through the aperture: the low four bits of the CPU byte become
/// a per-plane colour across all four maps, under the Bit Mask.
///
/// Spec: IBM Figure 2-73 "Write Mode 2".
#[test]
fn guest_reaches_write_mode_2_through_the_aperture() {
    let mut code = select_planar_a0000_window();
    code.extend(write_seq(VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES));
    code.extend(write_gc(VGA_GC_BIT_MASK, 0xFF));
    // Write mode 2.
    code.extend(write_gc(VGA_GC_MODE, 0x02));
    #[rustfmt::skip]
    code.extend_from_slice(&[
        0xB8, 0x00, 0xA0,       // MOV AX, 0xA000
        0x8E, 0xC0,             // MOV ES, AX
        0x31, 0xFF,             // XOR DI, DI
        0xB0, 0x05,             // MOV AL, 0b0101 — colour 5
        0x26, 0x88, 0x05,       // MOV ES:[DI], AL
        0xF4,                   // HLT
    ]);

    let m = run_guest(code);

    // Colour bit set → the whole byte under the mask; clear → zero.
    assert_eq!(m.vga.plane_byte(0, 0), Some(0xFF));
    assert_eq!(m.vga.plane_byte(1, 0), Some(0x00));
    assert_eq!(m.vga.plane_byte(2, 0), Some(0xFF));
    assert_eq!(m.vga.plane_byte(3, 0), Some(0x00));
}

/// The `0xB8000` text path is unchanged by routing the whole aperture.
///
/// The HELLO ROM and every existing text-mode test depend on a byte written at
/// `0xB8000` landing in the interleaved character/attribute buffer with no
/// Graphics Controller involvement, under the mode-3 reset defaults.
#[test]
fn text_mode_writes_at_b8000_are_unchanged_by_the_aperture_routing() {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0xB8,       // MOV AX, 0xB800
        0x8E, 0xC0,             // MOV ES, AX
        0x31, 0xFF,             // XOR DI, DI
        0xB0, b'H',             // MOV AL, 'H'
        0x26, 0x88, 0x05,       // MOV ES:[DI], AL
        0x47,                   // INC DI
        0xB0, 0x07,             // MOV AL, 0x07
        0x26, 0x88, 0x05,       // MOV ES:[DI], AL
        0x4F,                   // DEC DI
        0x26, 0x8A, 0x05,       // MOV AL, ES:[DI]
        0xBA, 0x02, 0x04,       // MOV DX, 0x0402
        0xEE,                   // OUT DX, AL
        0xF4,                   // HLT
    ];

    let m = run_guest(code.to_vec());

    assert_eq!(m.vga.char_at(0, 0), Some(b'H'));
    assert_eq!(m.vga.attr_at(0, 0), Some(0x07));
    assert_eq!(
        m.debug_text(),
        "H",
        "the read came back from the text buffer"
    );
    // Nothing reached plane memory: text accesses do not touch the latches.
    assert_eq!(m.vga.plane_byte(0, 0), Some(0x00));
}

/// With Miscellaneous Output RAM Enable clear the device claims nothing, and
/// the access falls through to RAM instead of faulting.
///
/// Spec: FreeVGA External Registers — Misc Output bit 1 disconnects display
/// memory from the CPU.
#[test]
fn ram_enable_clear_falls_through_to_physical_memory() {
    let mut code = select_planar_a0000_window();
    // Misc Output with RAM Enable clear (default 0x67 minus bit 1).
    code.extend(out_imm(0x03C2, 0x65));
    #[rustfmt::skip]
    code.extend_from_slice(&[
        0xB8, 0x00, 0xA0,       // MOV AX, 0xA000
        0x8E, 0xC0,             // MOV ES, AX
        0x31, 0xFF,             // XOR DI, DI
        0xB0, b'Z',             // MOV AL, 'Z'
        0x26, 0x88, 0x05,       // MOV ES:[DI], AL
        0xF4,                   // HLT
    ]);

    let m = run_guest(code);

    assert!(!m.vga.mmio_claims(VGA_WINDOW_A0000_BASE));
    assert!(VgaText::in_aperture(VGA_WINDOW_A0000_BASE));
    assert_eq!(m.vga.plane_byte(0, 0), Some(0x00), "no plane was written");
    assert_eq!(
        m.mem.read_u8(VGA_WINDOW_A0000_BASE),
        Ok(b'Z'),
        "the unclaimed access fell through to RAM"
    );
}
