//! Planar 16-color display fetch — BIOS modes `0Dh`, `0Eh`, `10h`, `12h`.
//!
//! # Spec refs
//!
//! - IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//!   (42G2193) chapter 2: the four maps are serialized in parallel, so one bit
//!   position across maps 0–3 forms a 4-bit index with map 0 supplying bit 0;
//!   Figure 2-72 Graphics Mode (Shift Register Interleave bit 5, 256-Color
//!   Shift Mode bit 6); Figure 2-74 Miscellaneous (Graphics/Alphanumeric bit 0,
//!   Chain Odd/Even bit 1); Figure 2-34 Map Selection (Chain 4); Figure 2-79
//!   Attribute Mode Control.
//! - FreeVGA: Attribute Controller — Attribute Mode Control `ATGE` and `8BIT`,
//!   Color Plane Enable ("setting a bit to 0 will force the corresponding color
//!   plane to 0"), Internal Palette, Color Select, `P54S`; Color Registers PEL
//!   Mask; CRT Controller — Start Address, Offset ("the starting scan line is
//!   increased by twice the value of this register multiplied by the current
//!   memory address size"), End Horizontal Display, Vertical Display Enable End
//!   plus the Overflow bits, Maximum Scan Line and Scan Doubling, Mode Control
//!   byte/word addressing, Underline Location `DW`.
//!
//! Expected pixel indices in these tests are computed from the spec — the
//! bit-plane weighting and the documented reset ATC palette — not from the
//! renderer's own output.

use devices::{
    PortDevice, VgaRenderMode, VgaText, VGA_ATC_ADDRESS_DATA, VGA_ATC_COLOR_PLANE_ENABLE,
    VGA_ATC_COLOR_SELECT, VGA_ATC_DEFAULTS, VGA_ATC_MODE_8BIT, VGA_ATC_MODE_ATGE,
    VGA_ATC_MODE_CONTROL, VGA_ATC_MODE_P54S, VGA_CRTC_DATA, VGA_CRTC_INDEX,
    VGA_CRTC_MAX_SCAN_DOUBLING, VGA_CRTC_MAX_SCAN_LINE, VGA_CRTC_MODE_BYTE_ADDRESSING,
    VGA_CRTC_MODE_CONTROL, VGA_CRTC_OFFSET, VGA_CRTC_OVERFLOW, VGA_CRTC_OVERFLOW_VDE_BIT8,
    VGA_CRTC_OVERFLOW_VDE_BIT9, VGA_CRTC_START_ADDR_HIGH, VGA_CRTC_START_ADDR_LOW,
    VGA_CRTC_UNDERLINE_LOCATION, VGA_DAC_PEL_MASK, VGA_GC_DATA, VGA_GC_INDEX,
    VGA_GC_MEMORY_MAP_A0000_64K, VGA_GC_MISC, VGA_GC_MISC_CHAIN_ODD_EVEN,
    VGA_GC_MISC_GRAPHICS_MODE, VGA_GC_MISC_MEMORY_MAP_SHIFT, VGA_GC_MODE, VGA_GC_MODE_SHIFT256,
    VGA_INPUT_STATUS_1, VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES,
    VGA_SEQ_MEMORY_MODE, VGA_SEQ_MEMORY_MODE_CHAIN4, VGA_SEQ_MEMORY_MODE_EXTENDED,
    VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
};

/// CRTC End Horizontal Display (`0x01`). Spec: FreeVGA CRT Controller.
const CRTC_HORIZONTAL_DISPLAY_END: u8 = 0x01;
/// CRTC Vertical Display Enable End (`0x12`). Spec: FreeVGA CRT Controller.
const CRTC_VERTICAL_DISPLAY_END: u8 = 0x12;
/// Graphics Mode Shift Register Interleave (`0x05` bit5). Spec: IBM Fig. 2-72.
const GC_MODE_SHIFT_INTERLEAVE: u8 = 0x20;

fn set_crtc(v: &mut VgaText, index: u8, value: u8) {
    v.port_write(VGA_CRTC_INDEX, 1, u32::from(index));
    v.port_write(VGA_CRTC_DATA, 1, u32::from(value));
}

fn set_seq(v: &mut VgaText, index: u8, value: u8) {
    v.port_write(VGA_SEQ_INDEX, 1, u32::from(index));
    v.port_write(VGA_SEQ_DATA, 1, u32::from(value));
}

fn set_gc(v: &mut VgaText, index: u8, value: u8) {
    v.port_write(VGA_GC_INDEX, 1, u32::from(index));
    v.port_write(VGA_GC_DATA, 1, u32::from(value));
}

fn set_atc(v: &mut VgaText, index: u8, value: u8) {
    v.port_read(VGA_INPUT_STATUS_1, 1);
    v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(index));
    v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(value));
}

/// Program the CRTC display-end registers for a given pixel geometry.
///
/// Spec: FreeVGA — End Horizontal Display holds the character-clock count minus
/// one (8 dots per clock in graphics), and Vertical Display Enable End holds the
/// displayed scan-line count minus one across `0x12` and Overflow bits 1 and 6.
fn set_display_end(v: &mut VgaText, width: usize, scan_lines: usize) {
    set_crtc(
        v,
        CRTC_HORIZONTAL_DISPLAY_END,
        (width / 8 - 1).try_into().expect("character clocks fit"),
    );
    let vde = scan_lines - 1;
    set_crtc(v, CRTC_VERTICAL_DISPLAY_END, (vde & 0xFF) as u8);
    let mut overflow = 0u8;
    if vde & 0x100 != 0 {
        overflow |= VGA_CRTC_OVERFLOW_VDE_BIT8;
    }
    if vde & 0x200 != 0 {
        overflow |= VGA_CRTC_OVERFLOW_VDE_BIT9;
    }
    set_crtc(v, VGA_CRTC_OVERFLOW, overflow);
}

/// The register values a BIOS mode `12h` set programs, for the fields this
/// device models: `0xA0000` 64 KB graphics window, planar (no chain-4, no
/// odd/even chaining, no 256-color or interleaved shift), a 4-bit attribute
/// through the Attribute Controller, byte addressing with an Offset of `0x28`
/// (an 80-byte row stride = 640 pixels), and a 640×480 display end.
fn program_mode12h(v: &mut VgaText) -> VgaText {
    v.planes.fill(0);
    set_gc(
        v,
        VGA_GC_MISC,
        VGA_GC_MISC_GRAPHICS_MODE | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
    );
    set_gc(v, VGA_GC_MODE, 0x00);
    set_seq(
        v,
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
    );
    set_seq(v, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES);
    set_atc(v, VGA_ATC_MODE_CONTROL, VGA_ATC_MODE_ATGE);
    set_crtc(v, VGA_CRTC_MODE_CONTROL, VGA_CRTC_MODE_BYTE_ADDRESSING);
    set_crtc(v, VGA_CRTC_UNDERLINE_LOCATION, 0x00);
    set_crtc(v, VGA_CRTC_OFFSET, 0x28);
    set_crtc(v, VGA_CRTC_MAX_SCAN_LINE, 0x00);
    set_display_end(v, 640, 480);
    v.clone()
}

fn planar() -> VgaText {
    let mut v = VgaText::new();
    program_mode12h(&mut v);
    v
}

/// Put one byte into each map at the same display byte offset.
fn write_planes(v: &mut VgaText, offset: usize, bytes: [u8; 4]) {
    for (plane, byte) in bytes.iter().enumerate() {
        assert!(v.set_plane_byte(plane, offset, *byte));
    }
}

/// The DAC index the reset ATC Internal Palette produces for a 4-bit color,
/// with Color Select `00h` and `P54S` clear.
///
/// Spec: FreeVGA Attribute Controller — the palette entry's 6 bits are the DAC
/// index; the reset palette is the IBM `00 01 02 03 04 05 14 07 38 39 3A 3B 3C
/// 3D 3E 3F` sequence.
fn palette(color: u8) -> u8 {
    VGA_ATC_DEFAULTS[usize::from(color & 0x0F)]
}

/// Spec: IBM Figure 2-74 / 2-72 / 2-34 and FreeVGA Attribute Mode Control — the
/// whole planar signature selects the 16-color fetch, and the geometry comes
/// from the CRTC display-end registers.
#[test]
fn the_planar_signature_selects_the_sixteen_color_fetch() {
    let mut v = VgaText::new();
    assert_eq!(v.render_mode(), VgaRenderMode::Text);

    program_mode12h(&mut v);
    assert!(v.is_planar16_programming());
    assert_eq!(v.render_mode(), VgaRenderMode::Graphics16Planar);
    assert_eq!(v.crtc_address_multiplier(), 1);
    assert_eq!(v.graphics_row_stride_bytes(), 80);

    let frame = v.render_frame(false).expect("planar 16-color renders");
    assert_eq!((frame.width, frame.height), (640, 480));
    assert_eq!(frame.pixels.len(), 640 * 480);
    assert_eq!(frame.mode, VgaRenderMode::Graphics16Planar);
}

/// Spec: IBM PS/2 Video Subsystems chapter 2 — one bit position across maps 0–3
/// is a 4-bit index, map 0 supplying bit 0; the most significant bit of each
/// byte is the leftmost pixel.
#[test]
fn one_bit_across_four_maps_is_one_four_bit_index() {
    let mut v = planar();
    // Pixel 0 lights map 0 only, pixel 1 map 1 only, and so on; pixel 4 lights
    // all four maps, pixels 5-7 none.
    write_planes(
        &mut v,
        0,
        [
            0b1000_1000, // map 0 -> index bit 0 at pixels 0 and 4
            0b0100_1000, // map 1 -> index bit 1 at pixels 1 and 4
            0b0010_1000, // map 2 -> index bit 2 at pixels 2 and 4
            0b0001_1000, // map 3 -> index bit 3 at pixels 3 and 4
        ],
    );

    let frame = v.render_frame(false).expect("planar renders");
    let expected = [1u8, 2, 4, 8, 15, 0, 0, 0];
    for (x, color) in expected.iter().enumerate() {
        assert_eq!(
            frame.index_at(x, 0),
            Some(palette(*color)),
            "pixel {x} should be color {color}"
        );
    }
}

/// Spec: FreeVGA CRTC Offset — the row stride is `Offset * 2 * memory address
/// size`, which is 80 bytes under the mode-12h byte-addressed programming.
#[test]
fn rows_are_one_offset_stride_apart() {
    let mut v = planar();
    // Row 0 byte 0 and row 1 byte 0 (= display byte 80) light map 2 only.
    write_planes(&mut v, 0, [0, 0, 0b1000_0000, 0]);
    write_planes(&mut v, 80, [0, 0, 0b0100_0000, 0]);

    let frame = v.render_frame(false).expect("planar renders");
    assert_eq!(frame.index_at(0, 0), Some(palette(4)));
    assert_eq!(frame.index_at(1, 0), Some(palette(0)));
    assert_eq!(frame.index_at(0, 1), Some(palette(0)));
    assert_eq!(frame.index_at(1, 1), Some(palette(4)));
}

/// Spec: FreeVGA CRT Controller Start Address High/Low — the counter value of
/// the first displayed address, turned into a byte address by the addressing
/// multiplier (1 here, because mode 12h uses byte addressing).
#[test]
fn the_start_address_moves_the_origin() {
    let mut v = planar();
    write_planes(&mut v, 160, [0b1000_0000, 0, 0, 0]);
    assert_eq!(
        v.render_frame(false).unwrap().index_at(0, 0),
        Some(palette(0))
    );

    // Two rows in: 2 * 80 bytes, and byte addressing means counter == byte.
    set_crtc(&mut v, VGA_CRTC_START_ADDR_HIGH, 0x00);
    set_crtc(&mut v, VGA_CRTC_START_ADDR_LOW, 160);
    assert_eq!(
        v.render_frame(false).unwrap().index_at(0, 0),
        Some(palette(1))
    );
}

/// Spec: FreeVGA Attribute Controller Color Plane Enable — "setting a bit to 0
/// will force the corresponding color plane to 0", applied before the Internal
/// Palette. Round 3 recorded this register as having no display effect; it does
/// now.
#[test]
fn color_plane_enable_forces_disabled_planes_to_zero() {
    let mut v = planar();
    // Pixel 0 is color 15 with all four maps set.
    write_planes(&mut v, 0, [0x80, 0x80, 0x80, 0x80]);
    assert_eq!(
        v.render_frame(false).unwrap().index_at(0, 0),
        Some(palette(15))
    );

    // Disable maps 2 and 3: the index drops to 0b0011.
    set_atc(&mut v, VGA_ATC_COLOR_PLANE_ENABLE, 0b0011);
    assert_eq!(v.atc_color_plane_enable(), 0b0011);
    assert_eq!(
        v.render_frame(false).unwrap().index_at(0, 0),
        Some(palette(3))
    );

    set_atc(&mut v, VGA_ATC_COLOR_PLANE_ENABLE, 0b0000);
    assert_eq!(
        v.render_frame(false).unwrap().index_at(0, 0),
        Some(palette(0))
    );
}

/// Spec: FreeVGA Attribute Controller — the 4-bit index passes the Internal
/// Palette, then Color Select supplies DAC bits 7:6 (and 5:4 under `P54S`), then
/// the PEL Mask is applied. This is the same chain the text fetch uses.
#[test]
fn the_internal_palette_color_select_and_pel_mask_compose() {
    let mut v = planar();
    write_planes(&mut v, 0, [0x80, 0x80, 0, 0]); // pixel 0 = color 3

    // Reprogram Internal Palette entry 3 to 0x2A.
    set_atc(&mut v, 0x03, 0x2A);
    assert_eq!(v.render_frame(false).unwrap().index_at(0, 0), Some(0x2A));

    // Color Select bits 3:2 supply DAC bits 7:6.
    set_atc(&mut v, VGA_ATC_COLOR_SELECT, 0b1100);
    assert_eq!(
        v.render_frame(false).unwrap().index_at(0, 0),
        Some(0xC0 | 0x2A)
    );

    // With P54S set, Color Select bits 1:0 replace palette bits 5:4.
    set_atc(
        &mut v,
        VGA_ATC_MODE_CONTROL,
        VGA_ATC_MODE_ATGE | VGA_ATC_MODE_P54S,
    );
    set_atc(&mut v, VGA_ATC_COLOR_SELECT, 0b1101);
    // palette 0x2A -> low nibble 0xA; bits 5:4 from Color Select 01b -> 0x10.
    assert_eq!(
        v.render_frame(false).unwrap().index_at(0, 0),
        Some(0xC0 | 0x10 | 0x0A)
    );

    // PEL Mask is last.
    v.port_write(VGA_DAC_PEL_MASK, 1, 0x0F);
    assert_eq!(v.render_frame(false).unwrap().index_at(0, 0), Some(0x0A));
}

/// Every planar mode number this fetch covers, with the geometry each BIOS mode
/// programs. Spec: FreeVGA End Horizontal Display / Vertical Display Enable End
/// / Maximum Scan Line Scan Doubling.
#[test]
fn the_four_planar_mode_numbers_produce_their_documented_geometry() {
    // (mode, width, displayed scan lines, scan doubling, expected height)
    let cases = [
        ("0Dh", 320usize, 400usize, true, 200usize),
        ("0Eh", 640, 400, true, 200),
        ("10h", 640, 350, false, 350),
        ("12h", 640, 480, false, 480),
    ];
    for (mode, width, scan_lines, doubling, height) in cases {
        let mut v = planar();
        set_crtc(
            &mut v,
            VGA_CRTC_MAX_SCAN_LINE,
            if doubling {
                VGA_CRTC_MAX_SCAN_DOUBLING
            } else {
                0
            },
        );
        set_display_end(&mut v, width, scan_lines);
        set_crtc(&mut v, VGA_CRTC_OFFSET, (width / 16) as u8);

        assert_eq!(v.render_mode(), VgaRenderMode::Graphics16Planar, "{mode}");
        assert_eq!(v.graphics16_frame_size(), (width, height), "{mode}");
        assert_eq!(v.graphics_row_stride_bytes(), width / 8, "{mode}");
        let frame = v.render_frame(false).expect("renders");
        assert_eq!((frame.width, frame.height), (width, height), "{mode}");
    }
}

/// Spec: FreeVGA Maximum Scan Line — bits 4:0 hold the scan lines per row minus
/// one, so a non-zero value means the CRTC repeats a memory row and the frame
/// has fewer distinct rows.
#[test]
fn maximum_scan_line_divides_the_displayed_scan_lines_into_rows() {
    let mut v = planar();
    set_display_end(&mut v, 640, 400);
    set_crtc(&mut v, VGA_CRTC_MAX_SCAN_LINE, 1); // two scan lines per row
    assert_eq!(v.crtc_scan_lines_per_row(), 2);
    assert_eq!(v.graphics16_frame_size(), (640, 200));

    // Scan doubling multiplies it again.
    set_crtc(
        &mut v,
        VGA_CRTC_MAX_SCAN_LINE,
        1 | VGA_CRTC_MAX_SCAN_DOUBLING,
    );
    assert_eq!(v.crtc_scan_lines_per_row(), 4);
    assert_eq!(v.graphics16_frame_size(), (640, 100));
}

/// The signature is exclusive: each field that names a different fetch keeps
/// the planar renderer from claiming the programming.
#[test]
fn other_graphics_programmings_are_still_unsupported() {
    // Mode 13h and "mode X" set 8BIT / C256 / Chain 4; CGA modes set Shift
    // Register Interleave and Chain Odd/Even.
    /// A named way to break one field of the planar signature.
    type SignatureBreak = (&'static str, fn(&mut VgaText));

    let cases: [SignatureBreak; 5] = [
        ("8BIT set (mode 13h / mode X)", |v| {
            set_atc(
                v,
                VGA_ATC_MODE_CONTROL,
                VGA_ATC_MODE_ATGE | VGA_ATC_MODE_8BIT,
            );
        }),
        ("C256 set", |v| set_gc(v, VGA_GC_MODE, VGA_GC_MODE_SHIFT256)),
        ("Shift Register Interleave set (CGA 04h/05h)", |v| {
            set_gc(v, VGA_GC_MODE, GC_MODE_SHIFT_INTERLEAVE)
        }),
        ("Chain Odd/Even set (CGA 04h-06h)", |v| {
            set_gc(
                v,
                VGA_GC_MISC,
                VGA_GC_MISC_GRAPHICS_MODE
                    | VGA_GC_MISC_CHAIN_ODD_EVEN
                    | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
            )
        }),
        ("Chain 4 set", |v| {
            set_seq(
                v,
                VGA_SEQ_MEMORY_MODE,
                VGA_SEQ_MEMORY_MODE_EXTENDED
                    | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE
                    | VGA_SEQ_MEMORY_MODE_CHAIN4,
            )
        }),
    ];

    for (name, break_signature) in cases {
        let mut v = planar();
        break_signature(&mut v);
        assert!(!v.is_planar16_programming(), "{name}");
        assert_eq!(v.render_mode(), VgaRenderMode::Unsupported, "{name}");
        assert!(v.render_frame(false).is_none(), "{name}");
    }
}

/// Model choice, stated so nobody mistakes it for hardware: the geometry comes
/// from the CRTC display-end registers, and a CRTC that has not been programmed
/// gives a degenerate frame rather than an invented default resolution.
#[test]
fn an_unprogrammed_crtc_gives_a_degenerate_frame_not_a_default_resolution() {
    let mut v = VgaText::new();
    v.planes.fill(0);
    set_gc(
        &mut v,
        VGA_GC_MISC,
        VGA_GC_MISC_GRAPHICS_MODE | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
    );
    set_seq(
        &mut v,
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
    );
    set_atc(&mut v, VGA_ATC_MODE_CONTROL, VGA_ATC_MODE_ATGE);

    assert_eq!(v.render_mode(), VgaRenderMode::Graphics16Planar);
    // End Horizontal Display and Vertical Display Enable End have no reset
    // default in this model, and Maximum Scan Line resets to 16 scan lines.
    assert_eq!(v.graphics16_frame_size(), (8, 1));
}

/// A planar frame converts to RGBA through the DAC exactly like a text or
/// mode-13h frame.
#[test]
fn planar_frames_expand_through_the_dac() {
    let mut v = planar();
    set_display_end(&mut v, 640, 8);
    write_planes(&mut v, 0, [0x80, 0x80, 0, 0]); // color 3 -> DAC index 0x03
    v.dac_ram[usize::from(palette(3))] = [0x3F, 0x00, 0x15];

    let frame = v.render_frame(false).expect("renders");
    assert_eq!(frame.index_at(0, 0), Some(palette(3)));

    let rgba = v.frame_rgba8(&frame);
    assert_eq!(rgba.len(), frame.pixels.len() * 4);
    assert_eq!(&rgba[..4], &[0xFF, 0x00, 0x55, 0xFF]);
}
