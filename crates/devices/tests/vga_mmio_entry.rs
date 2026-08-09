//! Device-level tests for the single guest-facing VGA display-memory MMIO
//! entry point (`VgaText::mmio_read_u8` / `VgaText::mmio_write_u8`).
//!
//! These cover the whole CPU-side pipeline in one call: Miscellaneous Output
//! RAM Enable gating, Graphics Controller Miscellaneous window decode,
//! Sequencer plane addressing, and the Graphics Controller read/write data
//! path with its four latches.
//!
//! Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
//! (Sep 1992) chapter 2 "VGA Function": Figure 2-75 Video Memory Assignments
//! (Memory Map Select windows), Figure 2-74 Miscellaneous, Figures 2-33 / 2-34
//! Sequencer Memory Mode addressing, Figure 2-29 Map Mask, Figures 2-71 / 2-72
//! / 2-73 Read Map Select / Graphics Mode / Write Mode Definitions. FreeVGA
//! External Registers for Miscellaneous Output bit1 RAM Enable.
//!
//! See `docs/vga-r2-mmio-entry-point.md`.

use devices::{
    PortDevice, VgaText, VGA_GC_DATA, VGA_GC_INDEX, VGA_GC_MEMORY_MAP_A0000_128K,
    VGA_GC_MEMORY_MAP_A0000_64K, VGA_GC_MEMORY_MAP_B0000_32K, VGA_GC_MEMORY_MAP_B8000_32K,
    VGA_GC_MISC, VGA_GC_MISC_GRAPHICS_MODE, VGA_GC_MISC_MEMORY_MAP_SHIFT, VGA_MISC_OUTPUT_DEFAULT,
    VGA_MISC_OUTPUT_WRITE, VGA_MISC_RAM_ENABLE, VGA_SEQ_DATA, VGA_SEQ_INDEX, VGA_SEQ_MAP_MASK,
    VGA_SEQ_MAP_MASK_PLANES, VGA_SEQ_MEMORY_MODE, VGA_SEQ_MEMORY_MODE_EXTENDED,
    VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE, VGA_TEXT_BASE, VGA_TEXT_END, VGA_TEXT_SIZE,
    VGA_WINDOW_A0000_BASE, VGA_WINDOW_B0000_BASE,
};

fn write_gc(vga: &mut VgaText, index: u8, value: u8) {
    vga.port_write(VGA_GC_INDEX, 1, u32::from(index));
    vga.port_write(VGA_GC_DATA, 1, u32::from(value));
}

fn write_seq(vga: &mut VgaText, index: u8, value: u8) {
    vga.port_write(VGA_SEQ_INDEX, 1, u32::from(index));
    vga.port_write(VGA_SEQ_DATA, 1, u32::from(value));
}

/// Select a Memory Map Select window with graphics mode and Chain Odd/Even clear.
fn select_window(vga: &mut VgaText, map_select: u8) {
    write_gc(
        vga,
        VGA_GC_MISC,
        VGA_GC_MISC_GRAPHICS_MODE | (map_select << VGA_GC_MISC_MEMORY_MAP_SHIFT),
    );
}

/// Planar addressing (Extended Memory | Odd/Even disable) with all maps enabled.
fn planar_all_maps(vga: &mut VgaText) {
    write_seq(
        vga,
        VGA_SEQ_MEMORY_MODE,
        VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
    );
    write_seq(vga, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES);
}

/// The aperture is the widest range the subsystem can ever decode: the legacy
/// `0xA0000`–`0xBFFFF` 128 KB video hole. Spec: IBM Figure 2-75.
#[test]
fn aperture_spans_the_legacy_video_hole() {
    assert_eq!(VgaText::aperture(), (VGA_WINDOW_A0000_BASE, VGA_TEXT_END));
    assert!(!VgaText::in_aperture(VGA_WINDOW_A0000_BASE - 1));
    assert!(VgaText::in_aperture(VGA_WINDOW_A0000_BASE));
    assert!(VgaText::in_aperture(VGA_TEXT_END - 1));
    assert!(!VgaText::in_aperture(VGA_TEXT_END));
}

/// Only the selected Memory Map Select window is claimed, and the claim is
/// gated by Misc Output RAM Enable. Spec: IBM Figure 2-75; FreeVGA Misc Output.
#[test]
fn mmio_claims_follow_memory_map_select_and_ram_enable() {
    let mut vga = VgaText::new();
    let probes = [
        VGA_WINDOW_A0000_BASE,
        VGA_WINDOW_B0000_BASE,
        VGA_TEXT_BASE,
        VGA_TEXT_END - 1,
    ];

    select_window(&mut vga, VGA_GC_MEMORY_MAP_A0000_128K);
    assert_eq!(
        probes.map(|addr| vga.mmio_claims(addr)),
        [true, true, true, true]
    );

    select_window(&mut vga, VGA_GC_MEMORY_MAP_A0000_64K);
    assert_eq!(
        probes.map(|addr| vga.mmio_claims(addr)),
        [true, false, false, false]
    );

    select_window(&mut vga, VGA_GC_MEMORY_MAP_B0000_32K);
    assert_eq!(
        probes.map(|addr| vga.mmio_claims(addr)),
        [false, true, false, false]
    );

    select_window(&mut vga, VGA_GC_MEMORY_MAP_B8000_32K);
    assert_eq!(
        probes.map(|addr| vga.mmio_claims(addr)),
        [false, false, true, true]
    );

    vga.port_write(
        VGA_MISC_OUTPUT_WRITE,
        1,
        u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_RAM_ENABLE),
    );
    assert_eq!(
        probes.map(|addr| vga.mmio_claims(addr)),
        [false, false, false, false]
    );
}

/// Every byte of the `0xB8000` text window behaves the same whether it is
/// reached through the guest Graphics Controller path or the host alphanumeric
/// helpers, in the reset (mode-03h) programming the HELLO ROM and the existing
/// text tests depend on.
///
/// This is the property that makes one display memory safe: with odd/even
/// addressing, Map Mask `0x03`, write mode 0 and read mode 0 with Graphics
/// Mode Host Odd/Even read addressing set, the Graphics Controller resolves a
/// text access to exactly the map and offset the host interleave view uses.
#[test]
fn text_window_guest_and_host_paths_address_the_same_bytes() {
    let mut host = VgaText::new();
    let mut guest = VgaText::new();

    for offset in 0..VGA_TEXT_SIZE {
        let addr = VGA_TEXT_BASE + offset as u64;
        let value = (offset % 251) as u8;
        assert!(host.write_u8(addr, value));
        assert!(guest.mmio_write_u8(addr, value));
        assert_eq!(guest.mmio_read_u8(addr), host.read_u8(addr), "{addr:#X}");
    }

    assert_eq!(host.planes, guest.planes);
    // A guest read now loads all four latches, because it really does go
    // through the Graphics Controller. The host helpers still do not.
    let last = VGA_TEXT_SIZE - 2;
    assert_eq!(
        guest.gc_latches,
        [
            host.plane_byte(0, last).unwrap(),
            host.plane_byte(1, last).unwrap(),
            host.plane_byte(2, last).unwrap(),
            host.plane_byte(3, last).unwrap(),
        ]
    );
    assert_eq!(host.gc_latches, [0; 4]);
}

/// Host text helpers see what the guest wrote through the unified entry point.
#[test]
fn text_window_write_is_visible_through_host_text_helpers() {
    let mut vga = VgaText::new();
    assert!(vga.mmio_write_u8(VGA_TEXT_BASE, b'X'));
    assert!(vga.mmio_write_u8(VGA_TEXT_BASE + 1, 0x1F));

    assert_eq!(vga.char_at(0, 0), Some(b'X'));
    assert_eq!(vga.attr_at(0, 0), Some(0x1F));
    assert_eq!(vga.mmio_read_u8(VGA_TEXT_BASE), Some(b'X'));
    assert_eq!(vga.mmio_read_u8(VGA_TEXT_BASE + 1), Some(0x1F));
}

/// Accesses outside the selected window are not claimed, so `MachineBus` can
/// fall through to open bus / PhysMem.
#[test]
fn unclaimed_addresses_are_reported_as_not_handled() {
    let mut vga = VgaText::new();
    for addr in [
        VGA_WINDOW_A0000_BASE,
        VGA_WINDOW_B0000_BASE,
        VGA_TEXT_BASE - 1,
        VGA_TEXT_END,
    ] {
        assert_eq!(vga.mmio_read_u8(addr), None, "read {addr:#X}");
        assert!(!vga.mmio_write_u8(addr, 0xFF), "write {addr:#X}");
    }
}

/// RAM Enable clear disables every CPU access, text window included.
#[test]
fn ram_disable_blocks_the_unified_entry_point() {
    let mut vga = VgaText::new();
    assert!(vga.mmio_write_u8(VGA_TEXT_BASE, b'A'));
    vga.port_write(
        VGA_MISC_OUTPUT_WRITE,
        1,
        u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_RAM_ENABLE),
    );

    assert_eq!(vga.mmio_read_u8(VGA_TEXT_BASE), None);
    assert!(!vga.mmio_write_u8(VGA_TEXT_BASE, b'B'));
    assert_eq!(vga.char_at(0, 0), Some(b'A'));
}

/// A graphics window reaches plane memory through the Graphics Controller:
/// a write honors Map Mask, and a read loads all four latches.
#[test]
fn graphics_window_routes_through_the_graphics_controller() {
    let mut vga = VgaText::new();
    planar_all_maps(&mut vga);
    select_window(&mut vga, VGA_GC_MEMORY_MAP_A0000_64K);
    write_seq(&mut vga, VGA_SEQ_MAP_MASK, 0b0101);

    assert!(vga.mmio_write_u8(VGA_WINDOW_A0000_BASE + 0x10, 0x5A));

    // Map 1 is masked out, so it keeps the 80×25 blank-screen attribute the
    // reset fill left in the one display memory.
    assert_eq!(vga.plane_byte(0, 0x10), Some(0x5A));
    assert_eq!(vga.plane_byte(1, 0x10), Some(0x07));
    assert_eq!(vga.plane_byte(2, 0x10), Some(0x5A));
    assert_eq!(vga.plane_byte(3, 0x10), Some(0x00));

    assert_eq!(vga.mmio_read_u8(VGA_WINDOW_A0000_BASE + 0x10), Some(0x5A));
    assert_eq!(vga.gc_latches, [0x5A, 0x07, 0x5A, 0x00]);
}

/// With the 128 KB window selected, `0xB8000` is display-memory offset
/// `0x18000`, wrapped to `0x8000` in a 64 KiB map — it is *not* offset 0.
///
/// The separate text buffer used to hide this: it served `0xB8000` from its
/// own offset 0 whatever window was programmed. With one display memory the
/// window base decides the offset, for the guest path and the host
/// alphanumeric view alike. Spec: IBM Figure 2-75 Video Memory Assignments.
#[test]
fn the_128k_window_places_b8000_at_its_own_display_offset() {
    let mut vga = VgaText::new();
    planar_all_maps(&mut vga);
    select_window(&mut vga, VGA_GC_MEMORY_MAP_A0000_128K);

    assert!(vga.mmio_write_u8(VGA_WINDOW_B0000_BASE, 0x11));
    assert!(vga.mmio_write_u8(VGA_TEXT_BASE, b'T'));

    // 0xB0000 is window offset 0x10000, which wraps to 0 in a 64 KiB map.
    let b0000_offset = vga.plane_offset(VGA_WINDOW_B0000_BASE).expect("in window");
    assert_eq!(b0000_offset, 0x0000);
    assert_eq!(vga.plane_byte(0, b0000_offset), Some(0x11));

    // 0xB8000 is window offset 0x18000, which wraps to 0x8000.
    let b8000_offset = vga.plane_offset(VGA_TEXT_BASE).expect("in window");
    assert_eq!(b8000_offset, 0x8000);
    assert_eq!(vga.plane_byte(0, b8000_offset), Some(b'T'));
    assert_eq!(vga.read_u8(VGA_TEXT_BASE), Some(b'T'));
    assert_eq!(vga.mmio_read_u8(VGA_WINDOW_B0000_BASE), Some(0x11));
    assert_eq!(vga.mmio_read_u8(VGA_TEXT_BASE), Some(b'T'));

    // The character generator still fetches CRTC counter 0 at map offset 0,
    // which is where the 0xB0000 write landed under this window.
    assert_eq!(vga.char_at(0, 0), Some(0x11));
}

/// The `0xB0000` 32 KB window decodes plane offsets relative to `0xA0000`,
/// matching `plane_access`. Spec: IBM Figure 2-75.
#[test]
fn b0000_window_offsets_are_relative_to_its_own_base() {
    let mut vga = VgaText::new();
    planar_all_maps(&mut vga);
    select_window(&mut vga, VGA_GC_MEMORY_MAP_B0000_32K);

    assert!(vga.mmio_write_u8(VGA_WINDOW_B0000_BASE + 4, 0x77));
    assert_eq!(vga.plane_offset(VGA_WINDOW_B0000_BASE + 4), Some(4));
    assert_eq!(vga.plane_byte(0, 4), Some(0x77));
    assert_eq!(vga.mmio_read_u8(VGA_WINDOW_B0000_BASE + 4), Some(0x77));
}
