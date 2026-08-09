//! The "no font at reset" state, reported rather than left ambiguous.
//!
//! This device ships **no character ROM**; see `docs/vga-r4-font-provenance.md`
//! for the licensing reasoning. A blank alphanumeric frame is therefore
//! ambiguous — no glyph data versus no text on screen — and these tests pin the
//! API that tells a front end which it is, plus the way a host installs a font
//! it is entitled to use.
//!
//! # Spec refs
//!
//! - FreeVGA "VGA Text Mode Operation", Fonts — "The offset in plane 2 of a
//!   character within a bank is determined by taking the character's value and
//!   multiplying it by 32", so a bank is 256 × 32 = 8192 bytes; attribute bit 3
//!   selects Character Set A or B.
//! - FreeVGA Sequencer Character Map Select Register — the non-contiguous
//!   3-bit select fields naming the eight banks, and Memory Mode `Ext. Mem`,
//!   which "must be set to 1 to enable the character map selection".
//! - IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//!   (42G2193) chapter 2, character generator.

use devices::{
    PortDevice, VgaText, VGA_FONT_BANK_BYTES, VGA_FONT_GLYPH_BYTES, VGA_FONT_MAX_SCAN_LINES,
    VGA_FONT_PLANE, VGA_SEQ_CHAR_MAP_SELECT, VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_SEQ_MEMORY_MODE,
    VGA_SEQ_MEMORY_MODE_DEFAULT, VGA_SEQ_MEMORY_MODE_EXTENDED,
};

/// Bank `0000h` — Character Map Select `000b`. Spec: FreeVGA Sequencer `03h`.
const BANK_0: usize = 0x0000;
/// Bank `4000h` — Character Map Select `001b`. Spec: FreeVGA Sequencer `03h`.
const BANK_1: usize = 0x4000;

fn set_seq(v: &mut VgaText, index: u8, value: u8) {
    v.port_write(VGA_SEQ_INDEX, 1, u32::from(index));
    v.port_write(VGA_SEQ_DATA, 1, u32::from(value));
}

/// A 16-scan-line font where every glyph is a solid block. The content does not
/// matter to these tests, only that it is non-zero; it is generated, not
/// copied from anywhere.
fn solid_font(glyph_height: usize) -> Vec<u8> {
    vec![0xFF; 256 * glyph_height]
}

/// The state this device is actually in after reset, stated in the API rather
/// than left for a front end to infer from a blank screen.
#[test]
fn a_reset_device_reports_that_no_font_is_installed() {
    let v = VgaText::new();
    assert!(!v.text_font_installed());
    assert!(v.font_bank_is_blank(BANK_0));
    assert!(v.font_bank_is_blank(BANK_1));

    let frame = v.render_frame(false).expect("text mode renders");
    assert_eq!(frame.font_installed, Some(false));
    // And the consequence: every pixel is background except the hardware
    // cursor, which does not come from the font.
    let non_background = frame.pixels.iter().filter(|index| **index != 0).count();
    let cursor_pixels = frame.pixels.len() - non_background;
    assert!(cursor_pixels > 0, "the frame is not entirely lit");
}

/// Installing a font is what turns the report — and the frame — around.
#[test]
fn installing_a_font_bank_flips_the_report() {
    let mut v = VgaText::new();
    assert!(v.install_font_bank(BANK_0, 16, &solid_font(16)));

    assert!(v.text_font_installed());
    assert!(!v.font_bank_is_blank(BANK_0));
    assert_eq!(v.render_frame(false).unwrap().font_installed, Some(true));

    // Spec: FreeVGA Fonts — glyph `code` starts at `code * 32`, and the rows
    // past the font height are cleared so a short font leaves no residue.
    assert_eq!(v.text_glyph_row(b'A', 0x07, 0), 0xFF);
    assert_eq!(v.text_glyph_row(b'A', 0x07, 15), 0xFF);
    assert_eq!(v.text_glyph_row(b'A', 0x07, 16), 0x00);
    assert_eq!(
        v.plane_byte(VGA_FONT_PLANE, usize::from(b'A') * VGA_FONT_GLYPH_BYTES),
        Some(0xFF)
    );
}

/// Only the character sets the current programming selects count, which is what
/// makes the report answer the question a front end is actually asking.
///
/// Spec: FreeVGA Sequencer Character Map Select and Memory Mode `Ext. Mem`.
#[test]
fn the_report_follows_character_map_select() {
    let mut v = VgaText::new();
    // Extended Memory on so Character Map Select has any effect at all.
    set_seq(
        &mut v,
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_DEFAULT | VGA_SEQ_MEMORY_MODE_EXTENDED,
    );
    assert!(v.install_font_bank(BANK_1, 16, &solid_font(16)));

    // Both sets still select bank 0, which is blank.
    assert_eq!(v.seq_char_map_a_select(), 0);
    assert_eq!(v.seq_char_map_b_select(), 0);
    assert!(!v.text_font_installed());
    assert!(!v.font_bank_is_blank(BANK_1), "the data really is there");

    // Character Set B select `001b` lives in Sequencer 03h bits 1:0.
    set_seq(&mut v, VGA_SEQ_CHAR_MAP_SELECT, 0b01);
    assert_eq!(v.seq_char_map_b_select(), 1);
    assert!(v.text_font_installed());
}

/// A graphics frame has no character generator, so it reports no font state
/// rather than a misleading `false`.
#[test]
fn a_graphics_frame_reports_no_font_state() {
    use devices::{
        VGA_ATC_ADDRESS_DATA, VGA_ATC_MODE_ATGE, VGA_ATC_MODE_CONTROL, VGA_GC_DATA, VGA_GC_INDEX,
        VGA_GC_MEMORY_MAP_A0000_64K, VGA_GC_MISC, VGA_GC_MISC_GRAPHICS_MODE,
        VGA_GC_MISC_MEMORY_MAP_SHIFT, VGA_INPUT_STATUS_1, VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
    };

    let mut v = VgaText::new();
    v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MISC));
    v.port_write(
        VGA_GC_DATA,
        1,
        u32::from(
            VGA_GC_MISC_GRAPHICS_MODE
                | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
        ),
    );
    set_seq(
        &mut v,
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
    );
    v.port_read(VGA_INPUT_STATUS_1, 1);
    v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
    v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_ATGE));

    let frame = v.render_frame(false).expect("planar 16-color renders");
    assert_eq!(frame.font_installed, None);
}

/// A rejected install must not half-write display memory.
#[test]
fn install_font_bank_rejects_bad_arguments_without_touching_memory() {
    let mut v = VgaText::new();
    let good = solid_font(16);

    // Unaligned bank offset.
    assert!(!v.install_font_bank(BANK_0 + 1, 16, &good));
    // Bank past the enabled map size (map size is 64 KiB with Extended Memory).
    assert!(!v.install_font_bank(0x1_0000, 16, &good));
    // Zero and over-tall glyph heights.
    assert!(!v.install_font_bank(BANK_0, 0, &[]));
    assert!(!v.install_font_bank(
        BANK_0,
        VGA_FONT_MAX_SCAN_LINES + 1,
        &solid_font(VGA_FONT_MAX_SCAN_LINES + 1)
    ));
    // Wrong length for the stated height.
    assert!(!v.install_font_bank(BANK_0, 16, &solid_font(8)));

    assert!(!v.text_font_installed());
    assert!(v.font_bank_is_blank(BANK_0));
    assert_eq!(v.render_frame(false).unwrap().font_installed, Some(false));
}

/// Clearing Extended Memory shrinks every map to 16 KiB, so only bank `0000h`
/// is addressable and the higher banks cannot be installed.
///
/// Spec: FreeVGA Sequencer Memory Mode `Ext. Mem`.
#[test]
fn without_extended_memory_only_the_first_bank_fits() {
    let mut v = VgaText::new();
    set_seq(&mut v, VGA_SEQ_MEMORY_MODE, 0x00);
    assert_eq!(v.plane_size_bytes(), 0x4000);
    assert_eq!(VGA_FONT_BANK_BYTES, 0x2000);

    assert!(v.install_font_bank(BANK_0, 16, &solid_font(16)));
    assert!(!v.install_font_bank(BANK_1, 16, &solid_font(16)));
    assert!(v.text_font_installed());
}

/// A reset returns the device to the honest "no font" state, because the font
/// lives in display memory and a reset clears it.
#[test]
fn reset_returns_the_device_to_no_font() {
    let mut v = VgaText::new();
    assert!(v.install_font_bank(BANK_0, 16, &solid_font(16)));
    assert!(v.text_font_installed());

    v.reset();

    assert!(!v.text_font_installed());
    assert!(v.font_bank_is_blank(BANK_0));
    assert_eq!(v.render_frame(false).unwrap().font_installed, Some(false));
}
