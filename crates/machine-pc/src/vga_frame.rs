//! Host VGA frame capture path (text + mode 13h), no guest LFB.
//!
//! Isolated module so parallel machine slices avoid fighting `lib.rs`.
//!
//! Spec: FreeVGA text / chain-4 display fetch already implemented in
//! `devices::VgaText::render_frame` / `frame_rgba8`. VBE honesty:
//! `PhysBasePtr` stays zero (`docs/vga-r5-vbe-banked-framebuffer.md`); this
//! path is the host linear view only.

use crate::Machine;
use devices::{VgaFrame, VgaRenderMode};

/// One host capture of the current VGA display fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostVgaFrame {
    /// DAC-index frame from [`devices::VgaText::render_frame`].
    pub frame: VgaFrame,
    /// RGBA8 expansion via [`devices::VgaText::frame_rgba8`].
    pub rgba8: Vec<u8>,
}

impl HostVgaFrame {
    /// Display-fetch mode that produced this frame.
    pub fn mode(&self) -> VgaRenderMode {
        self.frame.mode
    }

    /// Mirror of [`VgaFrame::font_installed`].
    pub fn font_installed(&self) -> Option<bool> {
        self.frame.font_installed
    }

    pub fn width(&self) -> usize {
        self.frame.width
    }

    pub fn height(&self) -> usize {
        self.frame.height
    }
}

impl Machine {
    /// Capture the current host VGA frame (DAC indices), if the programming is
    /// one this model renders (text, mode 13h chain-4, planar 16-color).
    ///
    /// Wraps [`devices::VgaText::render_frame`]. Returns `None` for
    /// [`VgaRenderMode::Unsupported`] rather than inventing pixels.
    pub fn capture_vga_frame(&self, blink_off_half: bool) -> Option<VgaFrame> {
        self.vga.render_frame(blink_off_half)
    }

    /// Capture RGBA8 pixels for the current host VGA frame.
    ///
    /// Wraps [`Self::capture_vga_frame`] + [`devices::VgaText::frame_rgba8`].
    pub fn capture_vga_rgba8(&self, blink_off_half: bool) -> Option<Vec<u8>> {
        let frame = self.capture_vga_frame(blink_off_half)?;
        Some(self.vga.frame_rgba8(&frame))
    }

    /// Capture DAC-index frame plus RGBA8 in one host view.
    ///
    /// Prefer this over inventing a guest `PhysBasePtr` LFB aperture — VBE mode
    /// info keeps LFB unavailable and `PhysBasePtr = 0`
    /// (`docs/vga-r7-host-frame.md`).
    pub fn capture_vga_host_frame(&self, blink_off_half: bool) -> Option<HostVgaFrame> {
        let frame = self.capture_vga_frame(blink_off_half)?;
        let rgba8 = self.vga.frame_rgba8(&frame);
        Some(HostVgaFrame { frame, rgba8 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::PortDevice;

    fn program_mode13(m: &mut Machine) {
        // Same minimal mode-13h signature as devices VBE / mode13 tests.
        m.vga.port_write(0x3CE, 1, 0x06);
        m.vga.port_write(0x3CF, 1, 0x05); // graphics, A0000 64K
        m.vga.port_write(0x3CE, 1, 0x05);
        m.vga.port_write(0x3CF, 1, 0x40); // C256
        m.vga.port_write(0x3C4, 1, 0x04);
        m.vga.port_write(0x3C5, 1, 0x0E); // Ext + OE disable + Chain4
        m.vga.port_read(0x3DA, 1);
        m.vga.port_write(0x3C0, 1, 0x10);
        m.vga.port_write(0x3C0, 1, 0x41); // ATGE | 8BIT
        m.vga.port_read(0x3DA, 1);
        m.vga.port_write(0x3C0, 1, 0x20); // PAS
    }

    /// Spec: FreeVGA text fetch — host path surfaces font_installed.
    #[test]
    fn host_frame_text_mode_reports_font_and_rgba_size() {
        let mut m = Machine::new(64 * 1024);
        assert_eq!(
            m.capture_vga_host_frame(false)
                .unwrap()
                .font_installed(),
            Some(false)
        );
        assert!(m.install_vga_bringup_font());
        assert!(m.vga.put_char(0, 0, b'A', 0x07));

        let host = m.capture_vga_host_frame(false).expect("text frame");
        assert_eq!(host.mode(), VgaRenderMode::Text);
        assert_eq!(host.font_installed(), Some(true));
        assert_eq!((host.width(), host.height()), (720, 400));
        assert_eq!(host.rgba8.len(), host.width() * host.height() * 4);
        assert!(host.rgba8.iter().any(|&b| b != 0));
    }

    /// Spec: FreeVGA / IBM chain-4 mode 13h — host path, no PhysBasePtr LFB.
    #[test]
    fn host_frame_mode13_matches_chain4_geometry() {
        let mut m = Machine::new(64 * 1024);
        program_mode13(&mut m);
        assert_eq!(m.vga.render_mode(), VgaRenderMode::Graphics256Chain4);

        let host = m.capture_vga_host_frame(false).expect("mode13 frame");
        assert_eq!(host.mode(), VgaRenderMode::Graphics256Chain4);
        assert_eq!(host.font_installed(), None);
        assert_eq!((host.width(), host.height()), (320, 200));
        assert_eq!(host.rgba8.len(), 320 * 200 * 4);

        // Honesty: guest VBE still advertises no LFB.
        let block = m.vga.vbe_mode_info_block_bytes(0x13).expect("mode 13h");
        let attrs = u16::from_le_bytes([block[0], block[1]]);
        assert_eq!(attrs & (1 << 7), 0, "no LFB bit");
        let phys = u32::from_le_bytes(block[40..44].try_into().unwrap());
        assert_eq!(phys, 0, "PhysBasePtr stays zero");
    }

    #[test]
    fn capture_helpers_agree() {
        let m = Machine::new(64 * 1024);
        let frame = m.capture_vga_frame(false).unwrap();
        let rgba = m.capture_vga_rgba8(false).unwrap();
        let host = m.capture_vga_host_frame(false).unwrap();
        assert_eq!(frame, host.frame);
        assert_eq!(rgba, host.rgba8);
    }
}
