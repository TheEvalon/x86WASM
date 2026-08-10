//! Host bring-up font path — procedural map-2 glyphs, honest `font_installed`.
//!
//! # Spec refs
//!
//! - FreeVGA "VGA Text Mode Operation", Fonts — plane 2 banks, `code * 32`
//!   stride, 256 glyphs per 8 KiB bank.
//! - IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//!   character generator (map 2 is guest/video-BIOS state, not a crate ROM).
//! - `docs/vga-r4-font-provenance.md` — why reset still installs nothing;
//!   `docs/vga-r7-font-install.md` — this bring-up path.

use devices::{
    vga_bringup_font_glyphs, VgaText, VGA_BRINGUP_FONT_BYTES, VGA_BRINGUP_FONT_HEIGHT,
    VGA_FONT_GLYPH_BYTES, VGA_FONT_PLANE,
};

#[test]
fn bringup_font_buffer_has_expected_layout() {
    let glyphs = vga_bringup_font_glyphs();
    assert_eq!(glyphs.len(), VGA_BRINGUP_FONT_BYTES);
    // Space stays blank so the default 80×25 fill remains visually empty.
    let space = usize::from(b' ') * VGA_BRINGUP_FONT_HEIGHT;
    assert!(glyphs[space..space + VGA_BRINGUP_FONT_HEIGHT]
        .iter()
        .all(|&b| b == 0));
    // Non-space identity marker on scan line 1.
    let a = usize::from(b'A') * VGA_BRINGUP_FONT_HEIGHT;
    assert_eq!(glyphs[a], 0x7E);
    assert_eq!(glyphs[a + 1], b'A');
    assert_eq!(glyphs[a + VGA_BRINGUP_FONT_HEIGHT - 1], 0x7E);
}

#[test]
fn install_bringup_font_reports_font_installed_and_draws_glyphs() {
    let mut v = VgaText::new();
    assert!(!v.text_font_installed());
    assert_eq!(v.render_frame(false).unwrap().font_installed, Some(false));

    assert!(v.install_bringup_font());
    assert!(v.text_font_installed());
    let frame = v.render_frame(false).expect("text frame");
    assert_eq!(frame.font_installed, Some(true));

    // Spec: FreeVGA Fonts — glyph `A` at `code * 32` in bank 0.
    assert_eq!(v.text_glyph_row(b'A', 0x07, 0), 0x7E);
    assert_eq!(v.text_glyph_row(b'A', 0x07, 1), b'A');
    assert_eq!(v.text_glyph_row(b' ', 0x07, 0), 0x00);
    assert_eq!(
        v.plane_byte(VGA_FONT_PLANE, usize::from(b'A') * VGA_FONT_GLYPH_BYTES),
        Some(0x7E)
    );

    // A non-space character produces lit pixels once the font is present.
    assert!(v.put_char(0, 0, b'A', 0x07));
    let lit = v.render_frame(false).unwrap();
    assert!(lit.pixels.iter().any(|&p| p != 0));
}

#[test]
fn reset_clears_bringup_font() {
    let mut v = VgaText::new();
    assert!(v.install_bringup_font());
    assert!(v.text_font_installed());
    v.reset();
    assert!(!v.text_font_installed());
    assert_eq!(v.render_frame(false).unwrap().font_installed, Some(false));
}
