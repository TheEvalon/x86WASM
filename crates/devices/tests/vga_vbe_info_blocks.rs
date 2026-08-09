//! VBE 2.0 host-side information blocks for modes this VGA model can render.
//!
//! # Spec refs
//!
//! - VESA BIOS Extension (VBE) Core Functions Standard Version 2.0 — Function
//!   00h `VbeInfoBlock` (512 bytes with `VBE2` signature), Function 01h
//!   `ModeInfoBlock` (256 bytes), Capabilities, ModeAttributes (D5 VGA
//!   compatibility, D6 windowing, D7 linear framebuffer), MemoryModel,
//!   PhysBasePtr.
//! - IBM PS/2 Video Subsystems / FreeVGA — register programmings behind BIOS
//!   modes `03h`, `0Dh`, `0Eh`, `10h`, `12h`, `13h`.

use devices::{PortDevice, VgaRenderMode, VgaText};

/// Modes listed in the VbeInfoBlock VideoModePtr list.
const SUPPORTED_MODES: [u16; 6] = [0x03, 0x0D, 0x0E, 0x10, 0x12, 0x13];

/// Spec: VBE 2.0 Function 00h — signature `VBE2`, version `0200h`, TotalMemory
/// in 64 KiB units (256 KiB → 4), Capabilities all clear, mode list terminated
/// by `FFFFh`.
#[test]
fn vbe_info_block_reports_vbe2_and_only_renderable_modes() {
    let v = VgaText::new();
    let block = v.vbe_info_block_bytes();
    assert_eq!(&block[0..4], b"VBE2");
    assert_eq!(u16::from_le_bytes([block[4], block[5]]), 0x0200);
    assert_eq!(
        &block[10..14],
        &[0, 0, 0, 0],
        "Capabilities must stay truthful"
    );
    assert_eq!(u16::from_le_bytes([block[18], block[19]]), 4);

    // Mode list is embedded after the fixed header region used by this model.
    let mut modes = Vec::new();
    let mut i = 34;
    while i + 1 < block.len() {
        let mode = u16::from_le_bytes([block[i], block[i + 1]]);
        if mode == 0xFFFF {
            break;
        }
        modes.push(mode);
        i += 2;
    }
    assert_eq!(modes, SUPPORTED_MODES.to_vec());
}

/// Spec: VBE 2.0 ModeAttributes — D7 clear (no LFB), D6 clear (windowing
/// available), PhysBasePtr zero for every mode this model advertises.
#[test]
fn mode_info_blocks_are_banked_vga_compatible_without_lfb() {
    let v = VgaText::new();
    for mode in SUPPORTED_MODES {
        let block = v
            .vbe_mode_info_block_bytes(mode)
            .unwrap_or_else(|| panic!("mode {mode:#x} must be described"));
        let attrs = u16::from_le_bytes([block[0], block[1]]);
        assert_eq!(attrs & (1 << 7), 0, "mode {mode:#x}: no LFB bit");
        assert_eq!(attrs & (1 << 6), 0, "mode {mode:#x}: windowing available");
        assert_eq!(attrs & (1 << 5), 0, "mode {mode:#x}: VGA compatible");
        assert_eq!(attrs & 1, 1, "mode {mode:#x}: supported");
        let phys = u32::from_le_bytes(block[40..44].try_into().unwrap());
        assert_eq!(phys, 0, "mode {mode:#x}: PhysBasePtr stays zero");
    }
    assert!(v.vbe_mode_info_block_bytes(0x101).is_none());
}

/// Mode 13h is the packed-pixel chain-4 programming the renderer drives.
#[test]
fn mode_13h_mode_info_matches_chain4_renderer() {
    let v = VgaText::new();
    let block = v.vbe_mode_info_block_bytes(0x13).expect("mode 13h");
    assert_eq!(u16::from_le_bytes([block[16], block[17]]), 320); // BytesPerScanLine
    assert_eq!(u16::from_le_bytes([block[18], block[19]]), 320); // XResolution
    assert_eq!(u16::from_le_bytes([block[20], block[21]]), 200); // YResolution
    assert_eq!(block[25], 8); // BitsPerPixel
    assert_eq!(block[27], 4); // MemoryModel packed pixel
    assert_eq!(u16::from_le_bytes([block[8], block[9]]), 0xA000); // WinASegment
}

/// Spec: VBE 2.0 — planar 16-color modes use MemoryModel planar (`03h`).
#[test]
fn planar_modes_report_planar_memory_model_and_crtc_geometry() {
    let v = VgaText::new();
    let m12 = v.vbe_mode_info_block_bytes(0x12).expect("mode 12h");
    assert_eq!(u16::from_le_bytes([m12[18], m12[19]]), 640);
    assert_eq!(u16::from_le_bytes([m12[20], m12[21]]), 480);
    assert_eq!(m12[24], 4); // NumberOfPlanes
    assert_eq!(m12[25], 4); // BitsPerPixel
    assert_eq!(m12[27], 3); // planar
}

/// Host linear view is the existing chain-4 renderer, not a guest PhysBasePtr.
#[test]
fn host_linear_framebuffer_aliases_chain4_render_frame() {
    let mut v = VgaText::new();
    // Minimal mode-13h signature (same fields as vga_planar / mode13 tests).
    v.port_write(0x3CE, 1, 0x06);
    v.port_write(0x3CF, 1, 0x05); // graphics, A0000 64K
    v.port_write(0x3CE, 1, 0x05);
    v.port_write(0x3CF, 1, 0x40); // C256
    v.port_write(0x3C4, 1, 0x04);
    v.port_write(0x3C5, 1, 0x0E); // Ext + OE disable + Chain4
    v.port_read(0x3DA, 1);
    v.port_write(0x3C0, 1, 0x10);
    v.port_write(0x3C0, 1, 0x41); // ATGE | 8BIT
    v.port_read(0x3DA, 1);
    v.port_write(0x3C0, 1, 0x20); // PAS

    assert_eq!(v.render_mode(), VgaRenderMode::Graphics256Chain4);
    let frame = v.vbe_host_linear_framebuffer(false).expect("host LFB view");
    assert_eq!(frame.mode, VgaRenderMode::Graphics256Chain4);
    assert_eq!((frame.width, frame.height), (320, 200));
    assert!(v.vbe_mode_info_block_bytes(0x13).is_some());
}
