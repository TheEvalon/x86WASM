//! VGA host bring-up font wiring for machine tests / CLI bring-up.
//!
//! Prefer this module over editing `lib.rs` for font install helpers so
//! parallel lanes avoid merge fights on the machine root.

use crate::Machine;

impl Machine {
    /// Install the procedural VGA bring-up font into map 2 bank 0.
    ///
    /// Wraps [`devices::VgaText::install_bringup_font`]. Reset still leaves no
    /// font; this is an explicit host action. See `docs/vga-r7-font-install.md`.
    pub fn install_vga_bringup_font(&mut self) -> bool {
        self.vga.install_bringup_font()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_install_vga_bringup_font_flips_report() {
        let mut m = Machine::new(64 * 1024);
        assert!(!m.vga.text_font_installed());
        assert!(m.install_vga_bringup_font());
        assert!(m.vga.text_font_installed());
        assert_eq!(
            m.vga.render_frame(false).unwrap().font_installed,
            Some(true)
        );
    }
}
