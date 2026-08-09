//! VGA color text-mode frame buffer MMIO stub (physical `0xB8000`) plus CRTC
//! index/data port stub (`0x3D4`/`0x3D5` color / `0x3B4`/`0x3B5` mono via Misc
//! IOAS), Sequencer index/data stub (`0x3C4`/`0x3C5`), Graphics Controller
//! index/data stub (`0x3CE`/`0x3CF`), Attribute Controller address/data
//! flip-flop stub (`0x3C0`/`0x3C1` + Input Status #1 flip-flop reset and status
//! bits at `0x3DA`/`0x3BA` via Misc IOAS), Miscellaneous Output Register stub
//! (`0x3C2` write / `0x3CC` readback), and DAC / PEL color RAM stub
//! (`0x3C7`/`0x3C8`/`0x3C9`) and PEL Mask (`0x3C6`).
//!
//! # Spec refs
//!
//! - IBM VGA / classic PC: color text frame buffer at physical `0xB8000`,
//!   80×25 cells, 2 bytes per cell (ASCII character, attribute).
//! - OSDev Text UI — memory layout for mode 03h text (char at even offset,
//!   attribute at odd); window commonly treated as 32 KiB (`0xB8000`–`0xBFFFF`).
//! - OSDev VGA Hardware / FreeVGA CRT Controller / IBM VGA — CRTC Address/Data
//!   at color `0x3D4`/`0x3D5` or mono `0x3B4`/`0x3B5` per Misc Output IOAS;
//!   standard VGA has 25 CRTC registers (indexes `0x00`–`0x18`). Cursor Start
//!   `0x0A` (bits 4:0 scanline start; bit5 Cursor Disable), Cursor End `0x0B`
//!   (bits 4:0 scanline end), Cursor Location High `0x0E` / Low `0x0F` (16-bit
//!   character address into the refresh buffer). Start Address High `0x0C` /
//!   Low `0x0D` (16-bit character address of the first displayed cell;
//!   mode-03h reset default `0x0000`). Maximum Scan Line `0x09`
//!   (bits 4:0 = character cell height − 1; bit5 Start Vertical Blanking bit9;
//!   bit6 Line Compare bit9; bit7 Scan Doubling; mode-03h reset default `0x0F`
//!   for 16 scanlines). Offset `0x13` (logical line width in words; mode-03h
//!   reset default `0x28` for 80-column text). Underline Location `0x14`
//!   (bits 4:0 underline scanline − 1; bit5 DIV4; bit6 DW; mode-03h reset
//!   default `0x1F`). Overflow `0x07` (FreeVGA high bits: VT/VDE/VRS/SVB bit8,
//!   Line Compare bit8 in bit4, VT/VDE/VRS bit9). Vertical Retrace End `0x11`
//!   bit7 Protect: when set, writes to indexes `0x00`–`0x07` are ignored except
//!   Overflow (`0x07`) bit4 (Line Compare bit8); indexes `>= 0x08` (including
//!   Start Address, Maximum Scan Line, Offset, and Underline Location) remain
//!   writable.
//! - OSDev VGA Hardware / FreeVGA Sequencer Registers — Address `0x3C4`, Data
//!   `0x3C5`; indexes `0x00`–`0x04` (Reset, Clocking Mode, Map Mask, Character
//!   Map Select, Memory Mode). Map Mask `0x02` (bits 3:0 enable write planes
//!   0–3; mode-03h reset default `0x03` = planes 0+1). Character Map Select
//!   `0x03` (font map A/B select; mode-03h reset default `0x00`). Memory Mode
//!   `0x04` (bit1 Extended Memory, bit2 Odd/Even, bit3 Chain-4; mode-03h reset
//!   default `0x02` = Extended Memory; odd/even + chain-4 clear).
//! - OSDev VGA Hardware / FreeVGA Graphics Registers — Address `0x3CE`, Data
//!   `0x3CF`; indexes `0x00`–`0x08` (Set/Reset, Enable Set/Reset, Color Compare,
//!   Data Rotate, Read Map Select, Graphics Mode, Miscellaneous, Color Don't
//!   Care, Bit Mask). Graphics Mode `0x05` (bits 1:0 Write Mode, bit3 Read Mode,
//!   bit4 Host Odd/Even, bit5 Shift Register Interleave, bit6 Shift256;
//!   mode-03h reset default `0x10` = Host Odd/Even). Miscellaneous `0x06` (bit0
//!   Graphics/Alphanumeric, bit1 Chain Odd/Even, bits 3:2 Memory Map Select;
//!   mode-03h reset default `0x0E` = Chain Odd/Even + `B8000` map). Bit Mask
//!   `0x08` (bits 7:0 select which data bits are written; mode-03h reset default
//!   `0xFF` = all bits enabled).
//! - OSDev VGA Hardware / FreeVGA Attribute Controller Registers — Address/Data
//!   at `0x3C0` (flip-flop), Data Read at `0x3C1`; indexes `0x00`–`0x14`
//!   (palette `0x00`–`0x0F`, Mode Control `0x10` with mode-03h reset default
//!   `0x0C`, Overscan Color `0x11` with mode-03h reset default `0x00`, Color
//!   Plane Enable `0x12` with mode-03h reset default `0x0F`, Horizontal PEL
//!   Panning `0x13` with mode-03h reset default `0x08` (9-dot zero-shift),
//!   Color Select `0x14` with mode-03h reset default `0x00`). Reading Input
//!   Status #1 (color `0x3DA` / mono `0x3BA`) resets the flip-flop to address
//!   state.
//! - OSDev VGA Hardware / FreeVGA External Registers — Input Status #1 read at
//!   `0x3DA` (color) / `0x3BA` (mono): bit0 Display Disabled (inverted
//!   display-enable; set during horizontal or vertical retrace), bit3 Vertical
//!   Retrace.
//! - OSDev VGA Hardware / FreeVGA / IBM VGA Miscellaneous Output Register —
//!   write port `0x3C2`, readback port `0x3CC` (write-only at `0x3C2`); bit0
//!   I/O Address Select (IOAS): `1` = color CRTC/status map (`0x3D4`/`0x3D5`,
//!   Input Status #1 `0x3DA`); `0` = mono map (CRTC `0x3B4`/`0x3B5`, Input
//!   Status #1 `0x3BA`); bit1 RAM Enable: when clear, CPU accesses to video RAM
//!   (text plane `0xB8000`–`0xBFFFF`) are disabled.
//! - OSDev VGA Hardware / FreeVGA Color Registers + DAC Operation / IBM VGA /
//!   RBIL — PEL Mask `0x3C6` (R/W, default `0xFF`; ANDed with the color index
//!   for each displayed pixel before DAC lookup), PEL Address Write Mode
//!   `0x3C8`, PEL Address Read Mode write / DAC State read `0x3C7`, PEL Data
//!   `0x3C9` (R→G→B, 6-bit, auto-increment after blue); 256×3 DAC RAM.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.6 / §21 VGA text mode.
//!
//! # Scope (this slice)
//!
//! - One display memory ([`VgaText::planes`]): four maps of
//!   [`VGA_PLANE_SIZE`] bytes serving guest MMIO, the host alphanumeric
//!   helpers, and the character generator alike
//! - Byte R/W over the `VGA_TEXT_BASE`…`VGA_TEXT_END` alphanumeric window;
//!   reset fills the first 80×25 cells with space + attribute `0x07`
//! - Helpers for tests (`char_at` / `attr_at` / `put_char`)
//! - Character generator and text-mode display fetch
//!   ([`VgaText::render_frame`] → [`VgaFrame`] of DAC indices)
//! - CRTC index/data: latch index / store/read register file on the IOAS-selected
//!   map (`0x3D4`/`0x3D5` color or `0x3B4`/`0x3B5` mono; shared file); cursor
//!   registers `0x0A`/`0x0B`/`0x0E`/`0x0F` have store/readback plus helpers for
//!   text-mode cursor character offset / row-col; Start Address High/Low
//!   `0x0C`/`0x0D` store/readback with [`VgaText::text_start_address`] /
//!   [`VgaText::text_start_plane_offset`] helpers and mode-03h reset default
//!   `0x0000` (Protect does not block; host `char_at`/`attr_at`/`put_char`
//!   viewport is relative to start; CPU `0xB8000` MMIO stays absolute); Offset
//!   `0x13` store/readback with mode-03h reset default `0x28` and
//!   [`VgaText::text_row_pitch_chars`] (words→character cells; host helpers use
//!   pitch as row stride; Protect does not block); Maximum Scan Line `0x09`
//!   store/readback with mode-03h reset default `0x0F` (Protect does not block);
//!   Overflow `0x07` store/readback with FreeVGA bit consts (under Protect only
//!   bit4 / Line Compare bit8 remains writable); Vertical Retrace End `0x11`
//!   bit7 Protect blocks writes to indexes `0x00`–`0x07` (Overflow bit4 still
//!   writable; no host cursor glyph render, max-scan glyph height, Line Compare
//!   split-screen, or CRTC timing)
//! - Sequencer index/data noop: latch index on `0x3C4`, store/read register file
//!   on `0x3C5` with mode-03h-class reset defaults; Map Mask `0x02` store/readback
//!   with mode-03h reset default `0x03`; Character Map Select `0x03` store/readback
//!   with mode-03h reset default `0x00`; Memory Mode `0x04` store/readback with
//!   mode-03h reset default `0x02` (no timing/plane write-enable/font-map/
//!   chain-4/odd-even/extended-memory side effects)
//! - Graphics Controller index/data: latch index on `0x3CE`, store/read
//!   register file on `0x3CF` with mode-03h-class reset defaults; Graphics Mode
//!   `0x05` store/readback with mode-03h reset default `0x10`; Miscellaneous
//!   `0x06` store/readback with mode-03h reset default `0x0E`; Bit Mask `0x08`
//!   store/readback with mode-03h reset default `0xFF`
//! - Sequencer Memory Mode / Map Mask plane address decode
//!   ([`VgaText::plane_access`] / [`VgaText::plane_write_mask`] /
//!   [`VgaText::plane_offset`]): Chain 4 (A1:A0 select the map), odd/even
//!   (even → maps 0+2, odd → maps 1+3), planar, Extended Memory map size
//! - Graphics Controller data path over [`VgaText::planes`]:
//!   [`VgaText::gc_read_u8`] loads the four [`VgaText::gc_latches`] and applies
//!   read mode 0 (Read Map Select, A1:A0 in Chain 4, or A0 substituted for
//!   Read Map Select bit0 when Graphics Mode bit4 Host Odd/Even read
//!   addressing is set) or read mode 1 (Color Compare / Color Don't Care);
//!   [`VgaText::gc_write_u8`] applies write modes 0–3 with Set/Reset + Enable
//!   Set/Reset, Data Rotate + Function Select (write mode 3 included), Bit
//!   Mask, and Map Mask plane write enables.
//! - Single guest-facing display-memory MMIO entry point
//!   ([`VgaText::mmio_read_u8`] / [`VgaText::mmio_write_u8`]) running RAM
//!   Enable gating → Memory Map Select window decode → plane addressing → the
//!   Graphics Controller data path in one call, with
//!   [`VgaText::aperture`] / [`VgaText::in_aperture`] describing the widest
//!   decodable range (`0xA0000`–`0xBFFFF`) and [`VgaText::mmio_claims`] the
//!   currently claimed sub-range. Every claimed access runs the Graphics
//!   Controller path over the single display memory in [`VgaText::planes`];
//!   there is no separate text buffer. See `docs/vga-r2-mmio-entry-point.md`
//!   and `docs/vga-r3-unified-display-memory.md`.
//! - Attribute Controller noop: address/data flip-flop on `0x3C0`, data read on
//!   `0x3C1`, flip-flop reset via Input Status #1 (active IOAS map); Mode Control
//!   `0x10` store/readback with mode-03h reset default `0x0C` + host text attr
//!   blink interpretation (bit3 BLINK → attr bit7 blink / bg bits 6:4; clear →
//!   16-color bg via bit7; [`VgaText::text_attr_fg_dac_index_for_phase`] for
//!   blink-off half); Overscan Color `0x11` store/readback with mode-03h reset
//!   default `0x00`; Color Plane Enable `0x12` store/readback with mode-03h
//!   reset default `0x0F`; Horizontal PEL Panning `0x13` store/readback with
//!   mode-03h reset default `0x08` + host [`VgaText::text_pel_pan`] (9-dot
//!   Pixel Shift Count → left-shift pels within the character cell for render;
//!   `char_at`/`attr_at`/`put_char` stay on the character grid); Color Select
//!   `0x14` store/readback with mode-03h reset default `0x00`; Internal Palette
//!   `0x00`–`0x0F`, Mode Control P54S, and Color Select compose the host text
//!   DAC address via [`VgaText::atc_palette_dac_index`] / fg/bg helpers
//!   (overscan-display, plane-enable display, host canvas render, and VR÷32
//!   blink timer remain out)
//! - Input Status #1: ATC flip-flop reset + deterministic display-enable /
//!   vertical-retrace status bits (read-phase counter); port selected by Misc
//!   Output IOAS (`0x3DA` color / `0x3BA` mono)
//! - Misc Output store/readback (`0x3C2`/`0x3CC`); IOAS bit remaps Input Status
//!   #1 and CRTC index/data ownership; RAM Enable (bit1) gates CPU text-plane
//!   `read_u8`/`write_u8` (not clock select)
//! - Graphics Controller Miscellaneous `0x06` decode: Memory Map Select
//!   (bits 3:2) selects the CPU display window (`A0000` 128 KB / `A0000` 64 KB
//!   / `B0000` 32 KB / `B8000` 32 KB) reported by [`VgaText::display_window`]
//!   and enforced by [`VgaText::owns_display_addr`], `read_u8` / `write_u8`,
//!   and the GC data path; Chain Odd/Even (bit1) is a second source of
//!   odd/even host addressing; Graphics/Alphanumeric (bit0) is tracked
//! - DAC / PEL store/readback: write index `0x3C8`, data `0x3C9` (R→G→B), read
//!   index write / state read `0x3C7`; 256×3 RAM with mode-03h-ish defaults
//! - PEL Mask `0x3C6` R/W store/readback (default `0xFF`) + display-path AND on
//!   host text attr→DAC index helpers ([`VgaText::display_dac_index`] /
//!   [`VgaText::text_attr_fg_dac_index`] / [`VgaText::text_attr_bg_dac_index`] /
//!   [`VgaText::display_dac_rgb`]); does **not** alter `0x3C9` palette
//!   programming (FreeVGA/RBIL/Abrash document mask on pixel-index lookup only)
//!
//! # Unsupported (explicit)
//!
//! - Shift Register Interleave and 256-Color Shift Mode have no effect
//! - The display fetch covers the alphanumeric (character generator) path
//!   only. Planar 16-color pixel serialization has no renderer; see
//!   [`VgaText::render_mode`], which reports
//!   [`VgaRenderMode::Unsupported`] rather than guessing. There is no VBE, no
//!   host display, and no timing-accurate raster
//! - No font is installed at reset, so a freshly reset device renders no
//!   glyphs (`docs/vga-r3-character-generator.md`)
//! - ATC / Sequencer / GC timing, plane-enable / overscan display side effects,
//!   map-mask, write-mode, read-map, or bitmask side effects on the text plane;
//!   Internal Palette + Color Select attr→DAC composition is on host text
//!   helpers; PEL pan is exposed as [`VgaText::text_pel_pan`] for host render
//!   (no canvas pixel shift yet); no vertical-retrace÷32 blink timer (host
//!   supplies phase)
//! - Hidden-DAC unlock via repeated `0x3C6` reads; host canvas pixel render
//! - CRTC-timed Input Status #1 accuracy, vertical-retrace IRQ, Feature Control
//!   diagnostic bits
//! - Full CRTC timing/blanking / Maximum Scan Line glyph-height side effects
//!   (Protect write-gate + Max Scan store/readback only; no scanline counters)
//! - Misc Output clock-select / polarity side effects (RAM Enable bit1 enforced
//!   on CPU text-plane helpers)
//! - Planar graphics, VBE, host canvas rendering, dirty tracking
//! - Font ROM / host rendering of the hardware cursor glyph from CRTC start/end
//!   scanlines (register state + offset helpers only)

use crate::PortDevice;

/// Physical base of color text frame buffer (IBM VGA mode 03h).
pub const VGA_TEXT_BASE: u64 = 0x000B_8000;
/// Exclusive end of the 32 KiB text plane (`0xB8000`–`0xBFFFF`).
pub const VGA_TEXT_END: u64 = 0x000C_0000;
/// Bytes in the text plane window.
pub const VGA_TEXT_SIZE: usize = (VGA_TEXT_END - VGA_TEXT_BASE) as usize;
/// Base of the widest CPU display aperture the video subsystem can decode.
///
/// Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
/// (Sep 1992) Figure 2-75 Video Memory Assignments — the largest Memory Map
/// Select window is `A0000` for 128 KB, so no smaller selection can reach
/// outside `0xA0000`–`0xBFFFF`. A bus that routes this whole range to
/// [`VgaText::mmio_read_u8`] / [`VgaText::mmio_write_u8`] sees every window
/// selection without re-registering ranges when Miscellaneous changes.
pub const VGA_APERTURE_BASE: u64 = VGA_WINDOW_A0000_BASE;
/// Exclusive end of the CPU display aperture (`0xA0000`–`0xBFFFF`).
pub const VGA_APERTURE_END: u64 = VGA_TEXT_END;
/// Size in bytes of the CPU display aperture (128 KiB).
pub const VGA_APERTURE_SIZE: usize = (VGA_APERTURE_END - VGA_APERTURE_BASE) as usize;
/// The aperture covers every Memory Map Select window (IBM Figure 2-75).
const _: () = assert!(
    VGA_APERTURE_BASE == 0x000A_0000
        && VGA_APERTURE_END == 0x000C_0000
        && VGA_APERTURE_SIZE == 0x2_0000
        && VGA_APERTURE_BASE <= VGA_WINDOW_B0000_BASE
        && VGA_TEXT_END <= VGA_APERTURE_END
);
/// Columns in default 80×25 text mode.
pub const VGA_TEXT_COLS: usize = 80;
/// Rows in default 80×25 text mode.
pub const VGA_TEXT_ROWS: usize = 25;
/// Bytes per character cell (char + attribute).
pub const VGA_CELL_BYTES: usize = 2;
/// Default attribute: light gray on black (classic BIOS text).
pub const VGA_DEFAULT_ATTR: u8 = 0x07;
/// Default fill character (ASCII space).
pub const VGA_DEFAULT_CHAR: u8 = b' ';

/// Color CRTC Address (index) Register. Spec: OSDev VGA Hardware / FreeVGA.
pub const VGA_CRTC_INDEX: u16 = 0x3D4;
/// Color CRTC Data Register.
pub const VGA_CRTC_DATA: u16 = 0x3D5;
/// Mono CRTC Address (index) Register — active when Misc Output IOAS selects mono.
///
/// Spec: FreeVGA / IBM VGA Misc Output bit0 (IOAS) — mono I/O map places CRTC
/// Address/Data at `0x3B4`/`0x3B5` (same register file as color).
pub const VGA_CRTC_INDEX_MONO: u16 = 0x3B4;
/// Mono CRTC Data Register — active when Misc Output IOAS selects mono.
pub const VGA_CRTC_DATA_MONO: u16 = 0x3B5;
/// Number of standard VGA CRTC registers (indexes `0x00`–`0x18`).
pub const VGA_CRTC_REG_COUNT: usize = 0x19;

/// CRTC Cursor Start Register index.
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — bits 4:0 = cursor start
/// scanline; bit5 = Cursor Disable (`CD`).
pub const VGA_CRTC_CURSOR_START: u8 = 0x0A;
/// CRTC Cursor End Register index.
///
/// Spec: FreeVGA / IBM VGA — bits 4:0 = cursor end scanline.
pub const VGA_CRTC_CURSOR_END: u8 = 0x0B;
/// CRTC Start Address High Register index.
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — high byte of the 16-bit
/// start address (character offset of the first displayed cell in the refresh
/// buffer). Protect (Vertical Retrace End bit7) does **not** block this index
/// (`>= 0x08`). Host text viewport helpers apply the combined start address;
/// CPU plane MMIO stays absolute.
pub const VGA_CRTC_START_ADDR_HIGH: u8 = 0x0C;
/// CRTC Start Address Low Register index.
///
/// Spec: FreeVGA / IBM VGA — low byte of the 16-bit start address.
pub const VGA_CRTC_START_ADDR_LOW: u8 = 0x0D;
/// Mode-03h-class Start Address High reset default (`0x00`).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — display starts at character
/// address `0` (High=`0x00`, Low=`0x00`).
pub const VGA_CRTC_START_ADDR_HIGH_DEFAULT: u8 = 0x00;
/// Mode-03h-class Start Address Low reset default (`0x00`).
pub const VGA_CRTC_START_ADDR_LOW_DEFAULT: u8 = 0x00;
const _: () = assert!(
    VGA_CRTC_START_ADDR_HIGH == 0x0C
        && VGA_CRTC_START_ADDR_LOW == 0x0D
        && VGA_CRTC_START_ADDR_HIGH_DEFAULT == 0x00
        && VGA_CRTC_START_ADDR_LOW_DEFAULT == 0x00
);
/// CRTC Cursor Location High Register index.
///
/// Spec: FreeVGA / IBM VGA — high byte of the 16-bit cursor character address.
pub const VGA_CRTC_CURSOR_LOC_HIGH: u8 = 0x0E;
/// CRTC Cursor Location Low Register index.
///
/// Spec: FreeVGA / IBM VGA — low byte of the 16-bit cursor character address.
pub const VGA_CRTC_CURSOR_LOC_LOW: u8 = 0x0F;
/// Cursor Start bit5 — Cursor Disable (`CD`).
///
/// Spec: FreeVGA CRT Controller — when set, hardware cursor is disabled.
pub const VGA_CRTC_CURSOR_DISABLE: u8 = 0x20;
/// Cursor Start/End scanline field mask (bits 4:0).
pub const VGA_CRTC_CURSOR_SCANLINE_MASK: u8 = 0x1F;
/// CRTC Overflow Register index.
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — index `0x07`. Holds the
/// high bits of several vertical timing fields plus Line Compare bit8:
/// - bit0 Vertical Total bit8
/// - bit1 Vertical Display End bit8
/// - bit2 Vertical Retrace Start bit8
/// - bit3 Start Vertical Blanking bit8
/// - bit4 Line Compare bit8 (writable under Protect; see
///   [`VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8`])
/// - bit5 Vertical Total bit9
/// - bit6 Vertical Display End bit9
/// - bit7 Vertical Retrace Start bit9
///
/// Store/readback only in this stub (no CRTC timing / split-screen side effects).
pub const VGA_CRTC_OVERFLOW: u8 = 0x07;
/// Overflow bit0 — Vertical Total bit8. Spec: FreeVGA.
pub const VGA_CRTC_OVERFLOW_VT_BIT8: u8 = 0x01;
/// Overflow bit1 — Vertical Display End bit8. Spec: FreeVGA.
pub const VGA_CRTC_OVERFLOW_VDE_BIT8: u8 = 0x02;
/// Overflow bit2 — Vertical Retrace Start bit8. Spec: FreeVGA.
pub const VGA_CRTC_OVERFLOW_VRS_BIT8: u8 = 0x04;
/// Overflow bit3 — Start Vertical Blanking bit8. Spec: FreeVGA.
pub const VGA_CRTC_OVERFLOW_START_VBLANK_BIT8: u8 = 0x08;
/// Overflow bit4 — Line Compare bit8; remains writable under Protect.
///
/// Spec: FreeVGA Vertical Retrace End Protect — indexes `0x00`–`0x07` ignore
/// writes when Protect is set, except this Overflow bit.
pub const VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8: u8 = 0x10;
/// Overflow bit5 — Vertical Total bit9. Spec: FreeVGA.
pub const VGA_CRTC_OVERFLOW_VT_BIT9: u8 = 0x20;
/// Overflow bit6 — Vertical Display End bit9. Spec: FreeVGA.
pub const VGA_CRTC_OVERFLOW_VDE_BIT9: u8 = 0x40;
/// Overflow bit7 — Vertical Retrace Start bit9. Spec: FreeVGA.
pub const VGA_CRTC_OVERFLOW_VRS_BIT9: u8 = 0x80;
const _: () = assert!(
    VGA_CRTC_OVERFLOW == 0x07
        && VGA_CRTC_OVERFLOW_VT_BIT8 == 0x01
        && VGA_CRTC_OVERFLOW_VDE_BIT8 == 0x02
        && VGA_CRTC_OVERFLOW_VRS_BIT8 == 0x04
        && VGA_CRTC_OVERFLOW_START_VBLANK_BIT8 == 0x08
        && VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8 == 0x10
        && VGA_CRTC_OVERFLOW_VT_BIT9 == 0x20
        && VGA_CRTC_OVERFLOW_VDE_BIT9 == 0x40
        && VGA_CRTC_OVERFLOW_VRS_BIT9 == 0x80
);
/// CRTC Vertical Retrace End Register index.
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — index `0x11`; bit7 is
/// CRTC Registers Protect Enable.
pub const VGA_CRTC_VERTICAL_RETRACE_END: u8 = 0x11;
/// Vertical Retrace End bit7 — CRTC Registers Protect Enable.
///
/// Spec: FreeVGA / IBM VGA — when set, writes to CRTC indexes `0x00`–`0x07`
/// are ignored (except Overflow bit4 / Line Compare bit8).
pub const VGA_CRTC_PROTECT: u8 = 0x80;
/// CRTC Maximum Scan Line Register index.
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — index `0x09`. Bits 4:0 =
/// Maximum Scan Line (character cell height − 1); bit5 = Start Vertical
/// Blanking bit9; bit6 = Line Compare bit9; bit7 = Scan Doubling. Protect
/// (Vertical Retrace End bit7) does **not** block this index (`>= 0x08`).
pub const VGA_CRTC_MAX_SCAN_LINE: u8 = 0x09;
/// Maximum Scan Line field mask (bits 4:0). Spec: FreeVGA.
pub const VGA_CRTC_MAX_SCAN_MASK: u8 = 0x1F;
/// Maximum Scan Line bit5 — Start Vertical Blanking bit9. Spec: FreeVGA.
pub const VGA_CRTC_MAX_SCAN_START_VBLANK_BIT9: u8 = 0x20;
/// Maximum Scan Line bit6 — Line Compare bit9. Spec: FreeVGA.
pub const VGA_CRTC_MAX_SCAN_LINE_COMPARE_BIT9: u8 = 0x40;
/// Maximum Scan Line bit7 — Scan Doubling. Spec: FreeVGA.
pub const VGA_CRTC_MAX_SCAN_DOUBLING: u8 = 0x80;
/// Mode-03h-class Maximum Scan Line reset default (`0x0F` = 16 scanlines − 1).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — 8×16 character cell uses
/// Maximum Scan Line bits 4:0 = `0x0F`; Start Vertical Blanking / Line Compare
/// high bits and Scan Doubling clear. Other CRTC indexes remain `0` on reset
/// until programmed (store/readback only; no glyph-height side effects).
pub const VGA_CRTC_MAX_SCAN_LINE_DEFAULT: u8 = 0x0F;
const _: () = assert!(
    (VGA_CRTC_MAX_SCAN_LINE_DEFAULT & VGA_CRTC_MAX_SCAN_MASK) == 0x0F
        && (VGA_CRTC_MAX_SCAN_LINE_DEFAULT
            & (VGA_CRTC_MAX_SCAN_START_VBLANK_BIT9
                | VGA_CRTC_MAX_SCAN_LINE_COMPARE_BIT9
                | VGA_CRTC_MAX_SCAN_DOUBLING))
            == 0
);
/// CRTC Offset Register index.
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — index `0x13`. Bits 7:0 =
/// Offset (logical line width of the screen, in words when byte addressing is
/// used). Protect (Vertical Retrace End bit7) does **not** block this index
/// (`>= 0x08`). Host text helpers convert words→character cells (`Offset * 2`)
/// for row stride ([`VgaText::text_row_pitch_chars`]).
pub const VGA_CRTC_OFFSET: u8 = 0x13;
/// Mode-03h-class Offset reset default (`0x28` = 40 words → 80 columns).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Offset `0x28` for 80-column
/// text (`0x28 * 2` = 80 character cells between adjacent rows).
pub const VGA_CRTC_OFFSET_DEFAULT: u8 = 0x28;
const _: () = assert!(VGA_CRTC_OFFSET_DEFAULT == 0x28 && VGA_CRTC_OFFSET == 0x13);
/// CRTC Underline Location Register index.
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — index `0x14`. Bits 4:0 =
/// Underline Location (scanline − 1 within the character cell); bit5 = Divide
/// Memory Address Clock by 4 (DIV4); bit6 = Double-Word Addressing (DW). Protect
/// (Vertical Retrace End bit7) does **not** block this index (`>= 0x08`).
/// DW/DIV4 addressing side effects and host underline rendering are out of
/// scope (store/readback only).
pub const VGA_CRTC_UNDERLINE_LOCATION: u8 = 0x14;
/// Underline Location field mask (bits 4:0). Spec: FreeVGA.
pub const VGA_CRTC_UNDERLINE_MASK: u8 = 0x1F;
/// Underline Location bit5 — Divide Memory Address Clock by 4 (DIV4). Spec: FreeVGA.
pub const VGA_CRTC_UNDERLINE_DIV4: u8 = 0x20;
/// Underline Location bit6 — Double-Word Addressing (DW). Spec: FreeVGA.
pub const VGA_CRTC_UNDERLINE_DW: u8 = 0x40;
/// Mode-03h-class Underline Location reset default (`0x1F`).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Underline Location bits 4:0 =
/// `0x1F` (scanline 32 − 1, past a 16-line cell so underline is off); DIV4 and
/// DW clear. Store/readback only; no host underline or DW/DIV4 side effects.
pub const VGA_CRTC_UNDERLINE_LOCATION_DEFAULT: u8 = 0x1F;
const _: () = assert!(
    (VGA_CRTC_UNDERLINE_LOCATION_DEFAULT & VGA_CRTC_UNDERLINE_MASK) == 0x1F
        && (VGA_CRTC_UNDERLINE_LOCATION_DEFAULT
            & (VGA_CRTC_UNDERLINE_DIV4 | VGA_CRTC_UNDERLINE_DW))
            == 0
);

/// Sequencer Address (index) Register.
///
/// Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — Sequencer Address at `0x3C4`,
/// Data at `0x3C5`.
pub const VGA_SEQ_INDEX: u16 = 0x3C4;
/// Sequencer Data Register.
pub const VGA_SEQ_DATA: u16 = 0x3C5;
/// Number of standard VGA Sequencer registers (indexes `0x00`–`0x04`).
pub const VGA_SEQ_REG_COUNT: usize = 5;

/// Mode-03h-class Sequencer reset defaults (store/readback only).
///
/// Spec: FreeVGA / IBM VGA alphanumeric programming SeaBIOS probes —
/// Reset `0x03` (both reset bits clear → run), Clocking Mode `0x00`,
/// Map Mask [`VGA_SEQ_MAP_MASK_DEFAULT`] (planes 0+1), Character Map Select
/// [`VGA_SEQ_CHAR_MAP_SELECT_DEFAULT`], Memory Mode
/// [`VGA_SEQ_MEMORY_MODE_DEFAULT`] (extended memory enable; odd/even + chain-4
/// clear).
pub const VGA_SEQ_DEFAULTS: [u8; VGA_SEQ_REG_COUNT] = [0x03, 0x00, 0x03, 0x00, 0x02];
/// Sequencer index: Clocking Mode register. Spec: FreeVGA Sequencer Registers.
pub const VGA_SEQ_CLOCKING_MODE: u8 = 0x01;
/// Clocking Mode bit0 — 8/9 Dot Mode (1 = 8 dots/char, 0 = 9). Spec: FreeVGA.
pub const VGA_SEQ_CLOCKING_8DOT: u8 = 0x01;
/// Default Clocking Mode has 9-dot characters (bit0 clear).
const _: () =
    assert!((VGA_SEQ_DEFAULTS[VGA_SEQ_CLOCKING_MODE as usize] & VGA_SEQ_CLOCKING_8DOT) == 0);
/// Sequencer Map Mask Register index.
///
/// Spec: FreeVGA Sequencer Registers / IBM VGA — index `0x02`. Bits 3:0 enable
/// write planes 0–3 (`1` = that plane may be written). Plane write-path
/// enforcement is out of scope (store/readback only).
pub const VGA_SEQ_MAP_MASK: u8 = 0x02;
/// Mode-03h-class Map Mask reset default (`0x03` = planes 0+1 enabled).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Map Mask `0x03` so host
/// writes update character and attribute planes. Store/readback only; no
/// map-mask side effects on the text plane.
pub const VGA_SEQ_MAP_MASK_DEFAULT: u8 = 0x03;
const _: () = assert!(
    VGA_SEQ_MAP_MASK == 0x02
        && VGA_SEQ_MAP_MASK_DEFAULT == 0x03
        && VGA_SEQ_DEFAULTS[VGA_SEQ_MAP_MASK as usize] == VGA_SEQ_MAP_MASK_DEFAULT
);
/// Sequencer Character Map Select Register index.
///
/// Spec: FreeVGA Sequencer Registers / IBM VGA — index `0x03`. Selects which
/// character-generator maps (fonts in plane 2) are used for alphanumeric
/// display (Map Select A/B fields). Font-map / glyph-fetch side effects are
/// out of scope (store/readback only).
pub const VGA_SEQ_CHAR_MAP_SELECT: u8 = 0x03;
/// Mode-03h-class Character Map Select reset default (`0x00` = map 0).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Character Map Select `0x00`
/// so both Map Select A and B refer to font map 0. Store/readback only; no
/// font-map side effects on glyph fetch.
pub const VGA_SEQ_CHAR_MAP_SELECT_DEFAULT: u8 = 0x00;
const _: () = assert!(
    VGA_SEQ_CHAR_MAP_SELECT == 0x03
        && VGA_SEQ_CHAR_MAP_SELECT_DEFAULT == 0x00
        && VGA_SEQ_DEFAULTS[VGA_SEQ_CHAR_MAP_SELECT as usize] == VGA_SEQ_CHAR_MAP_SELECT_DEFAULT
);
/// Sequencer Memory Mode Register index.
///
/// Spec: FreeVGA Sequencer Registers / IBM VGA — index `0x04`. Bit1 Extended
/// Memory, bit2 Odd/Even (host addressing), bit3 Chain-4. Chain-4 / odd-even /
/// extended-memory plane addressing side effects are out of scope
/// (store/readback only).
pub const VGA_SEQ_MEMORY_MODE: u8 = 0x04;
/// Mode-03h-class Memory Mode reset default (`0x02` = Extended Memory).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Memory Mode `0x02` (bit1
/// Extended Memory set; Odd/Even and Chain-4 clear). Store/readback only; no
/// chain-4 / odd-even / extended-memory side effects on the text plane.
pub const VGA_SEQ_MEMORY_MODE_DEFAULT: u8 = 0x02;
const _: () = assert!(
    VGA_SEQ_MEMORY_MODE == 0x04
        && VGA_SEQ_MEMORY_MODE_DEFAULT == 0x02
        && VGA_SEQ_DEFAULTS[VGA_SEQ_MEMORY_MODE as usize] == VGA_SEQ_MEMORY_MODE_DEFAULT
);
/// Memory Mode bit1 — Extended Memory (EM).
///
/// Spec: IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
/// (Sep 1992) Figure 2-33, Memory Mode Register index hex 04: "When set to 1,
/// the Extended Memory field (bit 1) enables the video memory from 64KB to
/// 256KB." Clear therefore leaves [`VGA_PLANE_SIZE_NO_EXTENDED`] addressable
/// per map instead of [`VGA_PLANE_SIZE`].
pub const VGA_SEQ_MEMORY_MODE_EXTENDED: u8 = 0x02;
/// Memory Mode bit2 — Odd/Even (OE); `0` selects odd/even host addressing.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-33: "When the Odd/Even field
/// (bit 2) is set to 0, even system addresses access maps 0 and 2, while odd
/// system addresses access maps 1 and 3. When set to 1, system addresses
/// sequentially access data within a bit map, and the maps are accessed
/// according to the value in the Map Mask register." OSDev VGA Hardware names
/// the same bit "Odd/Even Disable".
pub const VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE: u8 = 0x04;
/// Memory Mode bit3 — Chain 4 (CH4).
///
/// Spec: IBM PS/2 Video Subsystems Figures 2-33 / 2-34: when set, "the 2
/// low-order bits select the map accessed" (A1 A0 → map 0–3).
pub const VGA_SEQ_MEMORY_MODE_CHAIN4: u8 = 0x08;
/// Mode-03h default has Chain 4 clear, Odd/Even addressing on, Extended set.
const _: () = assert!(
    VGA_SEQ_MEMORY_MODE_DEFAULT & VGA_SEQ_MEMORY_MODE_CHAIN4 == 0
        && VGA_SEQ_MEMORY_MODE_DEFAULT & VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE == 0
        && VGA_SEQ_MEMORY_MODE_DEFAULT & VGA_SEQ_MEMORY_MODE_EXTENDED != 0
);

/// Number of VGA memory maps (planes).
///
/// Spec: IBM PS/2 Video Subsystems §2 "Graphics Controller" / Figure 2-15
/// 256KB Video Memory Map — four 64 KB maps.
pub const VGA_PLANE_COUNT: usize = 4;
/// Addressable bytes per map with Memory Mode Extended Memory set (256 KB total).
pub const VGA_PLANE_SIZE: usize = 0x1_0000;
/// Addressable bytes per map with Extended Memory clear (64 KB total).
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-33 documents the 64 KB / 256 KB
/// memory size, not what a host access above the 64 KB boundary does. This
/// emulator wraps the per-map offset within the enabled region as a
/// deterministic model choice (see `docs/vga-plane-memory-model.md`).
pub const VGA_PLANE_SIZE_NO_EXTENDED: usize = VGA_PLANE_SIZE / VGA_PLANE_COUNT;
/// All four map-enable bits of the Map Mask register.
pub const VGA_SEQ_MAP_MASK_PLANES: u8 = 0x0F;
const _: () = assert!(
    VGA_PLANE_COUNT == 4
        && VGA_PLANE_SIZE == 0x1_0000
        && VGA_PLANE_SIZE_NO_EXTENDED == 0x4000
        && VGA_SEQ_MAP_MASK_PLANES == 0x0F
);

/// Host-address → map (plane) addressing model currently programmed.
///
/// Spec: IBM PS/2 Video Subsystems Figures 2-33 / 2-34 (Sequencer Memory Mode)
/// and OSDev VGA Hardware "Addressing Logic".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VgaPlaneAddressing {
    /// Chain 4: A1/A0 select the map (Memory Mode bit3 set).
    Chain4,
    /// Odd/Even: even host addresses reach maps 0+2, odd addresses maps 1+3.
    OddEven,
    /// Planar: every map sees the same offset; Map Mask alone selects writes.
    Planar,
}

/// Decoded plane targets and per-map offset for one CPU display-window access.
///
/// Produced by [`VgaText::plane_access`]. `planes` is the address-logic result
/// before the Map Mask; `write_planes` is that value ANDed with the Map Mask
/// (OSDev VGA Hardware, Write Mode 0: "The Memory Plane Write Enable field is
/// ANDed with the input from the address logic").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VgaPlaneAccess {
    /// Maps selected by address decode alone (bit *n* = map *n*).
    pub planes: u8,
    /// Maps actually write-enabled (`planes` AND Map Mask).
    pub write_planes: u8,
    /// Byte offset within each selected map.
    pub offset: usize,
    /// Addressing model that produced this mapping.
    pub addressing: VgaPlaneAddressing,
}

/// Miscellaneous Output Register write port.
///
/// Spec: OSDev VGA Hardware / FreeVGA — MOR is write-only at `0x3C2`;
/// readback is at `0x3CC`.
pub const VGA_MISC_OUTPUT_WRITE: u16 = 0x3C2;
/// Miscellaneous Output Register readback port (`0x3CC`).
pub const VGA_MISC_OUTPUT_READ: u16 = 0x3CC;
/// Reset / BIOS text-mode-ish Misc Output default (`0x67`).
///
/// Spec: FreeVGA / OSDev Misc Output — common mode-03h programming selects
/// color CRTC map (IOAS), RAM enable, and 25 MHz-class clock select bits
/// (`0x67` = color + enable RAM + typical text-mode polarity/clock nibble).
pub const VGA_MISC_OUTPUT_DEFAULT: u8 = 0x67;
/// Miscellaneous Output bit0 — I/O Address Select (IOAS).
///
/// Spec: FreeVGA / IBM VGA Misc Output — `1` = color I/O map (`0x3Dx` CRTC +
/// Input Status #1 `0x3DA`); `0` = mono I/O map (`0x3Bx` CRTC + Input Status #1
/// `0x3BA`). This stub remaps CRTC index/data and Input Status #1 ownership.
pub const VGA_MISC_IOAS: u8 = 0x01;
/// Miscellaneous Output bit1 — RAM Enable ("Enable RAM").
///
/// Spec: FreeVGA External Registers / IBM VGA Misc Output — when clear, CPU
/// accesses to video RAM (including the color text plane at `0xB8000`–`0xBFFFF`)
/// are disabled. This stub gates [`VgaText::read_u8`] / [`VgaText::write_u8`]:
/// clear → same "not handled" returns as out-of-window (`None` / `false`) so
/// `MachineBus` falls through to open-bus / PhysMem; set (default in `0x67`) →
/// plane R/W unchanged.
pub const VGA_MISC_RAM_ENABLE: u8 = 0x02;
/// Misc Output bits 3:2 — Clock Select. Spec: FreeVGA / IBM VGA.
pub const VGA_MISC_CLOCK_SELECT: u8 = 0x0C;
/// Clock Select = 00b (25.175 MHz class). Spec: FreeVGA Misc Output.
pub const VGA_MISC_CLOCK_25MHZ: u8 = 0x00;
/// Clock Select = 01b (28.322 MHz class). Spec: FreeVGA Misc Output.
pub const VGA_MISC_CLOCK_28MHZ: u8 = 0x04;
/// Compile-time check: reset default Misc Output uses 28 MHz-class clock select.
const _: () = assert!((VGA_MISC_OUTPUT_DEFAULT & VGA_MISC_CLOCK_SELECT) == VGA_MISC_CLOCK_28MHZ);
/// Compile-time check: 25 MHz-class encoding is bits 3:2 = 00.
const _: () = assert!(VGA_MISC_CLOCK_25MHZ == 0x00);
/// Misc Output bit6 — Horizontal Sync Polarity. Spec: FreeVGA / IBM VGA.
pub const VGA_MISC_HSYNC_POLARITY: u8 = 0x40;
/// Misc Output bit7 — Vertical Sync Polarity. Spec: FreeVGA / IBM VGA.
pub const VGA_MISC_VSYNC_POLARITY: u8 = 0x80;

/// Graphics Controller Address (index) Register.
///
/// Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — Graphics Address at `0x3CE`,
/// Data at `0x3CF`.
pub const VGA_GC_INDEX: u16 = 0x3CE;
/// Graphics Controller Data Register.
pub const VGA_GC_DATA: u16 = 0x3CF;
/// Number of standard VGA Graphics Controller registers (indexes `0x00`–`0x08`).
pub const VGA_GC_REG_COUNT: usize = 9;
/// Graphics Controller Set/Reset Register index.
///
/// Spec: FreeVGA Graphics Registers / IBM VGA — index `0x00`. Bits 3:0 =
/// Set/Reset value written to enabled planes in Write Mode 0 when Enable
/// Set/Reset selects that plane. Plane write-path side effects are out of
/// scope (store/readback only).
pub const VGA_GC_SET_RESET: u8 = 0x00;
/// Mode-03h-class Set/Reset reset default (`0x00`).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Set/Reset `0x00`.
/// Store/readback only; no Set/Reset plane fill side effects on the text plane.
pub const VGA_GC_SET_RESET_DEFAULT: u8 = 0x00;
/// Graphics Controller Enable Set/Reset Register index.
///
/// Spec: FreeVGA Graphics Registers / IBM VGA — index `0x01`. Bits 3:0 enable
/// Set/Reset for planes 0–3 in Write Mode 0 (`1` = that plane uses Set/Reset
/// instead of CPU data). Plane write-path side effects are out of scope
/// (store/readback only).
pub const VGA_GC_ENABLE_SET_RESET: u8 = 0x01;
/// Mode-03h-class Enable Set/Reset reset default (`0x00`).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Enable Set/Reset `0x00`
/// (CPU data path). Store/readback only; no Enable Set/Reset side effects.
pub const VGA_GC_ENABLE_SET_RESET_DEFAULT: u8 = 0x00;
/// Graphics Controller Color Compare Register index.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-68 — bits 3:0 are the 4-bit color
/// compared against the four maps when read mode 1 is selected.
pub const VGA_GC_COLOR_COMPARE: u8 = 0x02;
/// Graphics Controller Read Map Select Register index.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-71 — bits 1:0 select the map used
/// for system read operations in read mode 0.
pub const VGA_GC_READ_MAP_SELECT: u8 = 0x04;
/// Read Map Select map field (bits 1:0).
pub const VGA_GC_READ_MAP_SELECT_MASK: u8 = 0x03;
/// Graphics Controller Color Don't Care Register index.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-76 — bit *n* set makes map *n*
/// participate in the read-mode-1 color compare.
pub const VGA_GC_COLOR_DONT_CARE: u8 = 0x07;
/// Graphics Controller Data Rotate / Function Select Register index.
///
/// Spec: FreeVGA Graphics Registers / IBM VGA — index `0x03`. Bits 2:0 =
/// rotate count; bits 4:3 = Function Select (`00` = replace/NOP, `01` = AND,
/// `10` = OR, `11` = XOR) applied to CPU latches vs write data. Rotate/ALU
/// side effects on the plane write path are out of scope (store/readback only).
pub const VGA_GC_DATA_ROTATE: u8 = 0x03;
/// Mode-03h-class Data Rotate reset default (`0x00` = no rotate, replace).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Data Rotate `0x00` so host
/// writes replace plane data without rotate or logical mix. Store/readback
/// only; no rotate/function side effects on the text plane.
pub const VGA_GC_DATA_ROTATE_DEFAULT: u8 = 0x00;
/// Data Rotate bits 2:0 — Rotate Count (write mode 0 right rotate).
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-69 RC.
pub const VGA_GC_ROTATE_COUNT_MASK: u8 = 0x07;
/// Data Rotate bits 4:3 — Function Select field.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-70 Operation Select Bit
/// Definitions: `00` unmodified, `01` AND, `10` OR, `11` XOR with latched data.
pub const VGA_GC_FUNCTION_SELECT_MASK: u8 = 0x18;
/// Function Select `00` — data unmodified.
pub const VGA_GC_FUNCTION_REPLACE: u8 = 0x00;
/// Function Select `01` — data ANDed with latched data.
pub const VGA_GC_FUNCTION_AND: u8 = 0x08;
/// Function Select `10` — data ORed with latched data.
pub const VGA_GC_FUNCTION_OR: u8 = 0x10;
/// Function Select `11` — data XORed with latched data.
pub const VGA_GC_FUNCTION_XOR: u8 = 0x18;
const _: () = assert!(
    VGA_GC_FUNCTION_SELECT_MASK
        == (VGA_GC_FUNCTION_REPLACE
            | VGA_GC_FUNCTION_AND
            | VGA_GC_FUNCTION_OR
            | VGA_GC_FUNCTION_XOR)
        && VGA_GC_ROTATE_COUNT_MASK == 0x07
        && VGA_GC_COLOR_COMPARE == 0x02
        && VGA_GC_READ_MAP_SELECT == 0x04
        && VGA_GC_READ_MAP_SELECT_MASK == 0x03
        && VGA_GC_COLOR_DONT_CARE == 0x07
);
/// Graphics Controller Graphics Mode Register index.
///
/// Spec: FreeVGA Graphics Registers / IBM VGA — index `0x05`. Bits 1:0 = Write
/// Mode (`00`–`11`); bit3 = Read Mode; bit4 = Host Odd/Even Memory Read
/// Addressing; bit5 = Shift Register Interleave; bit6 = 256-Color Shift Mode.
/// Write/read-mode and shift side effects on the plane path are out of scope
/// (store/readback only).
pub const VGA_GC_MODE: u8 = 0x05;
/// Mode-03h-class Graphics Mode reset default (`0x10` = Host Odd/Even).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Graphics Mode `0x10` so host
/// memory reads use odd/even addressing. Store/readback only; no write-mode /
/// read-mode / shift side effects on the text plane.
pub const VGA_GC_MODE_DEFAULT: u8 = 0x10;
/// Graphics Mode bits 1:0 — Write Mode field.
///
/// Spec: IBM PS/2 Video Subsystems Figures 2-72 / 2-73.
pub const VGA_GC_MODE_WRITE_MASK: u8 = 0x03;
/// Graphics Mode bit3 — Read Mode (`0` = map read, `1` = color compare).
pub const VGA_GC_MODE_READ: u8 = 0x08;
/// Graphics Mode bit4 — Host Odd/Even Memory Read Addressing Enable.
///
/// Spec: FreeVGA Graphics Registers, Graphics Mode "Host O/E": "When set to 1,
/// this bit selects the odd/even addressing mode used by the IBM
/// Color/Graphics Monitor Adapter. Normally, the value here follows the value
/// of Memory Mode register bit 2 in the sequencer." Sequencer Memory Mode
/// bit 2 governs odd/even *write* plane selection; this bit governs the
/// *read* side, so the host address bit A0 replaces bit 0 of Read Map Select
/// in read mode 0 ([`VgaText::gc_read_u8`]).
pub const VGA_GC_MODE_HOST_ODD_EVEN_READ: u8 = 0x10;
/// Mode-03h leaves Host Odd/Even read addressing set (CGA text emulation).
const _: () = assert!(
    VGA_GC_MODE_DEFAULT & VGA_GC_MODE_WRITE_MASK == 0
        && VGA_GC_MODE_DEFAULT & VGA_GC_MODE_READ == 0
        && VGA_GC_MODE_DEFAULT & VGA_GC_MODE_HOST_ODD_EVEN_READ != 0
);
/// Graphics Controller Miscellaneous Register index.
///
/// Spec: FreeVGA Graphics Registers / IBM VGA — index `0x06`. Bit0 = Graphics /
/// Alphanumeric Mode; bit1 = Chain Odd/Even Enable; bits 3:2 = Memory Map Select
/// (`00` = `A0000`–`BFFFF`, `01` = `A0000`–`AFFFF`, `10` = `B0000`–`B7FFF`,
/// `11` = `B8000`–`BFFFF`). Memory-map and chain-odd/even side effects on the
/// plane path are out of scope (store/readback only).
pub const VGA_GC_MISC: u8 = 0x06;
/// Mode-03h-class Miscellaneous reset default (`0x0E` = odd/even + `B8000` map).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Miscellaneous `0x0E`
/// (Chain Odd/Even + Memory Map Select `11` = `B8000`–`BFFFF`). Store/readback
/// only; no memory-map / chain-odd/even side effects on the text plane.
pub const VGA_GC_MISC_DEFAULT: u8 = 0x0E;
/// Miscellaneous bit0 — Graphics / Alphanumeric Mode.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-74 GM — set selects graphics modes
/// and disables the character generator latches. This model tracks the bit; it
/// has no display-path effect because there is no renderer.
pub const VGA_GC_MISC_GRAPHICS_MODE: u8 = 0x01;
/// Miscellaneous bit1 — Chain Odd/Even Enable.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-74 OE — "directs the system address
/// bit, A0, to be replaced by a higher-order bit. The odd map is then selected
/// when A0 is 1, and the even map when A0 is 0."
pub const VGA_GC_MISC_CHAIN_ODD_EVEN: u8 = 0x02;
/// Miscellaneous bits 3:2 — Memory Map Select.
pub const VGA_GC_MISC_MEMORY_MAP_MASK: u8 = 0x0C;
/// Shift of the Memory Map Select field within Miscellaneous.
pub const VGA_GC_MISC_MEMORY_MAP_SHIFT: u32 = 2;
/// Memory Map Select `00` — `0xA0000` for 128 KB. Spec: IBM Figure 2-75.
pub const VGA_GC_MEMORY_MAP_A0000_128K: u8 = 0b00;
/// Memory Map Select `01` — `0xA0000` for 64 KB. Spec: IBM Figure 2-75.
pub const VGA_GC_MEMORY_MAP_A0000_64K: u8 = 0b01;
/// Memory Map Select `10` — `0xB0000` for 32 KB. Spec: IBM Figure 2-75.
pub const VGA_GC_MEMORY_MAP_B0000_32K: u8 = 0b10;
/// Memory Map Select `11` — `0xB8000` for 32 KB. Spec: IBM Figure 2-75.
pub const VGA_GC_MEMORY_MAP_B8000_32K: u8 = 0b11;
/// Base of the `0xA0000` display windows.
pub const VGA_WINDOW_A0000_BASE: u64 = 0x000A_0000;
/// Base of the `0xB0000` 32 KB display window.
pub const VGA_WINDOW_B0000_BASE: u64 = 0x000B_0000;
/// Mode-03h default selects `0xB8000` for 32 KB with Chain Odd/Even set.
const _: () = assert!(
    (VGA_GC_MISC_DEFAULT & VGA_GC_MISC_MEMORY_MAP_MASK) >> VGA_GC_MISC_MEMORY_MAP_SHIFT
        == VGA_GC_MEMORY_MAP_B8000_32K
        && VGA_GC_MISC_DEFAULT & VGA_GC_MISC_CHAIN_ODD_EVEN != 0
        && VGA_GC_MISC_DEFAULT & VGA_GC_MISC_GRAPHICS_MODE == 0
        && VGA_WINDOW_A0000_BASE < VGA_WINDOW_B0000_BASE
        && VGA_WINDOW_B0000_BASE < VGA_TEXT_BASE
);
/// Graphics Controller Bit Mask Register index.
///
/// Spec: FreeVGA Graphics Registers / IBM VGA — index `0x08`. Bits 7:0 select
/// which bits of the CPU write data participate in plane updates (`1` = that
/// bit may be written). Plane write-path enforcement is out of scope
/// (store/readback only).
pub const VGA_GC_BIT_MASK: u8 = 0x08;
/// Mode-03h-class Bit Mask reset default (`0xFF` = all bits enabled).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Bit Mask `0xFF` so host
/// writes update every bit position. Store/readback only; no bitmask side
/// effects on the text plane.
pub const VGA_GC_BIT_MASK_DEFAULT: u8 = 0xFF;
const _: () = assert!(
    VGA_GC_SET_RESET == 0x00
        && VGA_GC_SET_RESET_DEFAULT == 0x00
        && VGA_GC_DEFAULTS[VGA_GC_SET_RESET as usize] == VGA_GC_SET_RESET_DEFAULT
        && VGA_GC_ENABLE_SET_RESET == 0x01
        && VGA_GC_ENABLE_SET_RESET_DEFAULT == 0x00
        && VGA_GC_DEFAULTS[VGA_GC_ENABLE_SET_RESET as usize] == VGA_GC_ENABLE_SET_RESET_DEFAULT
        && VGA_GC_DATA_ROTATE == 0x03
        && VGA_GC_DATA_ROTATE_DEFAULT == 0x00
        && VGA_GC_MODE == 0x05
        && VGA_GC_MODE_DEFAULT == 0x10
        && VGA_GC_DEFAULTS[VGA_GC_MODE as usize] == VGA_GC_MODE_DEFAULT
        && VGA_GC_MISC == 0x06
        && VGA_GC_MISC_DEFAULT == 0x0E
        && VGA_GC_DEFAULTS[VGA_GC_MISC as usize] == VGA_GC_MISC_DEFAULT
        && VGA_GC_BIT_MASK == 0x08
        && VGA_GC_BIT_MASK_DEFAULT == 0xFF
);

/// Mode-03h-class Graphics Controller reset defaults (store/readback only).
///
/// Spec: FreeVGA Graphics Registers / OSDev VGA Hardware / IBM VGA mode-03h —
/// SeaBIOS-class text programming: Set/Reset [`VGA_GC_SET_RESET_DEFAULT`],
/// Enable Set/Reset [`VGA_GC_ENABLE_SET_RESET_DEFAULT`], Color Compare `0x00`,
/// Data Rotate [`VGA_GC_DATA_ROTATE_DEFAULT`], Read Map Select `0x00`, Graphics
/// Mode [`VGA_GC_MODE_DEFAULT`] (host odd/even), Miscellaneous
/// [`VGA_GC_MISC_DEFAULT`] (odd/even + memory map `B8000`), Color Don't Care
/// `0x00`, Bit Mask [`VGA_GC_BIT_MASK_DEFAULT`].
pub const VGA_GC_DEFAULTS: [u8; VGA_GC_REG_COUNT] = [
    VGA_GC_SET_RESET_DEFAULT,
    VGA_GC_ENABLE_SET_RESET_DEFAULT,
    0x00,
    VGA_GC_DATA_ROTATE_DEFAULT,
    0x00,
    VGA_GC_MODE_DEFAULT,
    VGA_GC_MISC_DEFAULT,
    0x00,
    VGA_GC_BIT_MASK_DEFAULT,
];

/// Attribute Controller Address/Data Register (flip-flop).
///
/// Spec: FreeVGA Attribute Controller Registers / Accessing the Attribute
/// Registers / IBM VGA — write index then data to `0x3C0`; read index from
/// `0x3C0`; read data from `0x3C1`.
pub const VGA_ATC_ADDRESS_DATA: u16 = 0x3C0;
/// Attribute Controller Data Read Register.
pub const VGA_ATC_DATA_READ: u16 = 0x3C1;
/// Color Input Status #1 — reading resets the ATC address/data flip-flop and
/// returns display/retrace status bits when Misc Output IOAS selects color.
///
/// Spec: FreeVGA External Registers / IBM VGA / OSDev VGA Hardware — port
/// `0x3DA` (color); mono alias is [`VGA_INPUT_STATUS_1_MONO`].
pub const VGA_INPUT_STATUS_1: u16 = 0x3DA;
/// Mono Input Status #1 alias — same status model + ATC flip-flop reset when
/// Misc Output IOAS selects mono.
///
/// Spec: FreeVGA External Registers / IBM VGA / Misc Output bit0 (IOAS) —
/// mono I/O map places Input Status #1 at `0x3BA`.
pub const VGA_INPUT_STATUS_1_MONO: u16 = 0x3BA;
/// Input Status #1 bit0 — Display Disabled (inverted display-enable).
///
/// Spec: FreeVGA External Registers — set during horizontal or vertical retrace.
pub const VGA_STATUS1_DD: u8 = 0x01;
/// Input Status #1 bit3 — Vertical Retrace.
///
/// Spec: FreeVGA External Registers — set during the vertical retrace interval.
pub const VGA_STATUS1_VR: u8 = 0x08;
/// Period of the deterministic Input Status #1 read-phase counter.
///
/// Model choice (documented, not CRTC-timed): even phases are display-active
/// (`DD=0`, `VR=0`); odd phases are vertical retrace (`DD=1`, `VR=1`). Advances
/// only on active-map status reads (`0x3DA` or `0x3BA` per IOAS) so SeaBIOS-style
/// wait-for-VR / wait-for-end-VR loops terminate without a machine tick hook.
pub const VGA_STATUS1_PHASE_PERIOD: u8 = 2;
/// Number of standard VGA Attribute Controller registers (`0x00`–`0x14`).
pub const VGA_ATC_REG_COUNT: usize = 0x15;
/// PAS bit in the Attribute Address register (bit 5).
pub const VGA_ATC_PAS: u8 = 0x20;
/// Reset / BIOS text-mode-ish Attribute Address default (`PAS=1`, index 0).
///
/// Spec: Ralf Brown Interrupt List / IBM VGA — index register often left with
/// Palette Address Source set after mode programming.
pub const VGA_ATC_INDEX_DEFAULT: u8 = VGA_ATC_PAS;
/// Attribute Controller Mode Control Register index.
///
/// Spec: FreeVGA Attribute Controller Registers / IBM VGA — index `0x10`.
/// Selects graphics/alphanumeric mode, Line Graphics Enable (LGE/ELG), blink
/// enable, and related Attribute Controller display controls. Host text helpers
/// apply bit3 (BLINK) to attribute interpretation and bit7 (P54S) to Internal
/// Palette / Color Select DAC-address composition. Horizontal PEL Panning is
/// index `0x13` ([`VgaText::text_pel_pan`]).
pub const VGA_ATC_MODE_CONTROL: u8 = 0x10;
/// Mode Control bit3 — Blink Enable (FreeVGA `BLINK`).
///
/// Spec: FreeVGA Attribute Mode Control — when set, attribute bit7 enables
/// character blink and background intensity bit is forced off (bg bits 6:4);
/// when clear, attribute bit7 selects background intensity (16 bg colors).
pub const VGA_ATC_MODE_BLINK: u8 = 0x08;
/// Mode Control bit7 — Palette Bits 5-4 Select (FreeVGA `P54S`).
///
/// Spec: FreeVGA Attribute Mode Control — clear selects Internal Palette bits
/// 5:4 for DAC address bits 5:4; set replaces them with Color Select bits 1:0.
pub const VGA_ATC_MODE_P54S: u8 = 0x80;
/// Mode-03h-class Mode Control reset default (`0x0C` = BLINK|LGE, alphanumeric).
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-03h — Mode Control `0x0C` enables
/// Line Graphics Enable and blink for alphanumeric text. Host text attr→DAC
/// helpers honor [`VGA_ATC_MODE_BLINK`], P54S, and Color Select composition.
pub const VGA_ATC_MODE_CONTROL_DEFAULT: u8 = 0x0C;
/// Attribute Controller Overscan Color Register index.
///
/// Spec: FreeVGA Attribute Controller Registers / IBM VGA — index `0x11`.
/// Selects the color used for the overscan (border) region around the active
/// display. Overscan display side effects are out of scope (store/readback only).
pub const VGA_ATC_OVERSCAN_COLOR: u8 = 0x11;
/// Mode-03h-class Overscan Color reset default (`0x00` = black border).
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-03h — Overscan Color `0x00`.
/// Store/readback only; no overscan-display side effects.
pub const VGA_ATC_OVERSCAN_COLOR_DEFAULT: u8 = 0x00;
/// Attribute Controller Color Plane Enable Register index.
///
/// Spec: FreeVGA Attribute Controller Registers / IBM VGA — index `0x12`.
/// Bits 3:0 enable color planes 0–3 (display path). Plane-enable display side
/// effects are out of scope (store/readback only).
pub const VGA_ATC_COLOR_PLANE_ENABLE: u8 = 0x12;
/// Mode-03h-class Color Plane Enable reset default (`0x0F` = all planes on).
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-03h — Color Plane Enable `0x0F`.
/// Store/readback only; no plane-enable display side effects.
pub const VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT: u8 = 0x0F;
/// Attribute Controller Horizontal PEL Panning Register index.
///
/// Spec: FreeVGA Attribute Controller Registers / IBM VGA — index `0x13`.
/// Bits 3:0 = Pixel Shift Count. In 9-dot alphanumeric text, programmed value
/// `8` selects a zero-pixel left shift; values `0`..=`7` select a left shift of
/// `n+1` pels within the character cell (soft-scroll sequence). Host helpers
/// expose the decoded shift via [`VgaText::text_pel_pan`]; canvas pixel render
/// remains out of scope.
pub const VGA_ATC_HORIZONTAL_PEL_PANNING: u8 = 0x13;
/// Mode-03h-class Horizontal PEL Panning reset default (`0x08` = 9-dot zero-shift).
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-03h — Horizontal PEL Panning `0x08`
/// (9-dot text: shift-count encoding maps `8` → 0 pels). Host
/// [`VgaText::text_pel_pan`] applies this decode.
pub const VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT: u8 = 0x08;
/// Attribute Controller Color Select Register index.
///
/// Spec: FreeVGA Attribute Controller Registers / IBM VGA — index `0x14`.
/// Bits 1:0 = Color Select 5:4 and bits 3:2 = Color Select 7:6. Bits 3:2 always
/// supply DAC address bits 7:6. When [`VGA_ATC_MODE_P54S`] is set, bits 1:0
/// replace Internal Palette bits 5:4; otherwise palette bits 5:4 pass through.
pub const VGA_ATC_COLOR_SELECT: u8 = 0x14;
/// Mode-03h-class Color Select reset default (`0x00`).
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-03h — Color Select `0x00`.
pub const VGA_ATC_COLOR_SELECT_DEFAULT: u8 = 0x00;
const VGA_ATC_PALETTE_LOW_MASK: u8 = 0x0F;
const VGA_ATC_COLOR_SELECT_54_MASK: u8 = 0x03;
const VGA_ATC_COLOR_SELECT_76_MASK: u8 = 0x0C;

/// Mode-03h-class Attribute Controller reset defaults.
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-set palette — internal palette
/// `00/01/02/03/04/05/14/07/38/39/3A/3B/3C/3D/3E/3F`; Mode Control
/// [`VGA_ATC_MODE_CONTROL_DEFAULT`] (BLINK|LGE, alphanumeric; host text helpers
/// apply [`VGA_ATC_MODE_BLINK`]); Overscan Color
/// [`VGA_ATC_OVERSCAN_COLOR_DEFAULT`]; Color Plane Enable
/// [`VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT`]; Horizontal PEL Panning
/// [`VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT`]; Color Select
/// [`VGA_ATC_COLOR_SELECT_DEFAULT`].
pub const VGA_ATC_DEFAULTS: [u8; VGA_ATC_REG_COUNT] = [
    0x00,
    0x01,
    0x02,
    0x03,
    0x04,
    0x05,
    0x14,
    0x07,
    0x38,
    0x39,
    0x3A,
    0x3B,
    0x3C,
    0x3D,
    0x3E,
    0x3F,
    VGA_ATC_MODE_CONTROL_DEFAULT,
    VGA_ATC_OVERSCAN_COLOR_DEFAULT,
    VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT,
    VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT,
    VGA_ATC_COLOR_SELECT_DEFAULT,
];
const _: () = assert!(
    VGA_ATC_MODE_CONTROL == 0x10
        && VGA_ATC_MODE_BLINK == 0x08
        && VGA_ATC_MODE_P54S == 0x80
        && VGA_ATC_MODE_CONTROL_DEFAULT == 0x0C
        && (VGA_ATC_MODE_CONTROL_DEFAULT & VGA_ATC_MODE_BLINK) == VGA_ATC_MODE_BLINK
        && (VGA_ATC_MODE_CONTROL_DEFAULT & VGA_ATC_MODE_P54S) == 0
        && VGA_ATC_DEFAULTS[VGA_ATC_MODE_CONTROL as usize] == VGA_ATC_MODE_CONTROL_DEFAULT
);
const _: () = assert!(
    VGA_ATC_OVERSCAN_COLOR == 0x11
        && VGA_ATC_OVERSCAN_COLOR_DEFAULT == 0x00
        && VGA_ATC_DEFAULTS[VGA_ATC_OVERSCAN_COLOR as usize] == VGA_ATC_OVERSCAN_COLOR_DEFAULT
);
const _: () = assert!(
    VGA_ATC_COLOR_PLANE_ENABLE == 0x12
        && VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT == 0x0F
        && VGA_ATC_DEFAULTS[VGA_ATC_COLOR_PLANE_ENABLE as usize]
            == VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
);
const _: () = assert!(
    VGA_ATC_HORIZONTAL_PEL_PANNING == 0x13
        && VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT == 0x08
        && VGA_ATC_DEFAULTS[VGA_ATC_HORIZONTAL_PEL_PANNING as usize]
            == VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
);
const _: () = assert!(
    VGA_ATC_COLOR_SELECT == 0x14
        && VGA_ATC_COLOR_SELECT_DEFAULT == 0x00
        && VGA_ATC_DEFAULTS[VGA_ATC_COLOR_SELECT as usize] == VGA_ATC_COLOR_SELECT_DEFAULT
);

/// DAC / PEL Mask Register (R/W).
///
/// Spec: FreeVGA Color Registers / OSDev VGA Hardware / IBM VGA / RBIL —
/// ANDed with the color index of each displayed pixel before DAC lookup.
/// Default `0xFF` (no masking). Host text helpers apply the AND on the
/// attr→DAC index path; does not affect [`VGA_DAC_DATA`] palette programming
/// (datasheets describe display-path lookup only).
pub const VGA_DAC_PEL_MASK: u16 = 0x3C6;
/// Reset / power-on default for [`VGA_DAC_PEL_MASK`].
pub const VGA_DAC_PEL_MASK_DEFAULT: u8 = 0xFF;
/// DAC Address Read Mode write / DAC State read port.
///
/// Spec: FreeVGA Color Registers / OSDev VGA Hardware / IBM VGA — write sets
/// the read index for subsequent [`VGA_DAC_DATA`] reads; read returns DAC state.
pub const VGA_DAC_READ_INDEX: u16 = 0x3C7;
/// DAC Address Write Mode Register (R/W).
///
/// Spec: FreeVGA Color Registers — write sets the write index for subsequent
/// [`VGA_DAC_DATA`] writes; read returns the current write index.
pub const VGA_DAC_WRITE_INDEX: u16 = 0x3C8;
/// DAC / PEL Data Register (R/W) — R→G→B, auto-increment after blue.
pub const VGA_DAC_DATA: u16 = 0x3C9;
/// Number of DAC palette entries (256 × RGB).
pub const VGA_DAC_ENTRY_COUNT: usize = 256;
/// VGA DAC color components are 6-bit (bits 5:0).
pub const VGA_DAC_COLOR_MASK: u8 = 0x3F;
/// DAC State (read `0x3C7`): prepared to accept reads from PEL Data.
///
/// Spec: FreeVGA Color Registers — bits 1:0 = `00`.
pub const VGA_DAC_STATE_READ: u8 = 0x00;
/// DAC State (read `0x3C7`): prepared to accept writes to PEL Data.
///
/// Spec: FreeVGA Color Registers — bits 1:0 = `11`.
pub const VGA_DAC_STATE_WRITE: u8 = 0x03;

/// Mode-03h-class DAC reset defaults for indices `0`–`15` (6-bit RGB).
///
/// Spec: IBM VGA / classic CGA–EGA 16-color palette in 6-bit DAC units
/// (`0x00`/`0x15`/`0x2A`/`0x3F`). Indices `16`–`255` reset to black. Store /
/// readback only — host text attr→DAC uses ATC Internal Palette then these
/// entries; no host canvas render yet.
pub const VGA_DAC_CGA16_DEFAULTS: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], // 0  black
    [0x00, 0x00, 0x2A], // 1  blue
    [0x00, 0x2A, 0x00], // 2  green
    [0x00, 0x2A, 0x2A], // 3  cyan
    [0x2A, 0x00, 0x00], // 4  red
    [0x2A, 0x00, 0x2A], // 5  magenta
    [0x2A, 0x15, 0x00], // 6  brown
    [0x2A, 0x2A, 0x2A], // 7  light gray
    [0x15, 0x15, 0x15], // 8  dark gray
    [0x15, 0x15, 0x3F], // 9  light blue
    [0x15, 0x3F, 0x15], // 10 light green
    [0x15, 0x3F, 0x3F], // 11 light cyan
    [0x3F, 0x15, 0x15], // 12 light red
    [0x3F, 0x15, 0x3F], // 13 light magenta
    [0x3F, 0x3F, 0x15], // 14 yellow
    [0x3F, 0x3F, 0x3F], // 15 white
];

/// Build mode-03h-ish 256×3 DAC RAM (CGA-16 + black remainder).
pub fn vga_dac_default_ram() -> [[u8; 3]; VGA_DAC_ENTRY_COUNT] {
    let mut ram = [[0u8; 3]; VGA_DAC_ENTRY_COUNT];
    for (i, rgb) in VGA_DAC_CGA16_DEFAULTS.iter().enumerate() {
        ram[i] = *rgb;
    }
    ram
}

// ---------------------------------------------------------------------------
// Display fetch (character generator / renderer)
// ---------------------------------------------------------------------------

/// Map that supplies character codes during a text-mode display fetch.
///
/// Spec: FreeVGA "VGA Text Mode Operation", Display Memory Organization —
/// "Each byte in plane 0 is used to store an index into the character font map."
pub const VGA_TEXT_CHAR_PLANE: usize = 0;
/// Map that supplies attribute bytes during a text-mode display fetch.
///
/// Spec: FreeVGA "VGA Text Mode Operation" — "The corresponding byte in plane 1
/// is used to specify the attributes of the character".
pub const VGA_TEXT_ATTR_PLANE: usize = 1;
/// Map that holds the character generator font banks.
///
/// Spec: FreeVGA "VGA Text Mode Operation", Fonts — "Display plane 2 is used to
/// store the bitmaps for the characters themselves."
pub const VGA_FONT_PLANE: usize = 2;

/// Bytes per glyph in a font bank.
///
/// Spec: FreeVGA Fonts — "Each character is on a 32 byte boundary and is 32
/// bytes long."
pub const VGA_FONT_GLYPH_BYTES: usize = 32;
/// Bytes per font bank (256 glyphs × 32 bytes).
///
/// Spec: FreeVGA Fonts — "Display memory plane 2 is divided up into eight 8K
/// banks of characters, each of which holds 256 character bitmaps."
pub const VGA_FONT_BANK_BYTES: usize = 256 * VGA_FONT_GLYPH_BYTES;
const _: () = assert!(VGA_FONT_BANK_BYTES == 0x2000);
/// Maximum glyph height the character generator can address (32 scan lines).
///
/// Spec: FreeVGA Fonts — "Fonts are either 8 or 9 pixels wide and can be from 1
/// to 32 pixels high."
pub const VGA_FONT_MAX_SCAN_LINES: usize = VGA_FONT_GLYPH_BYTES;

/// Sequencer Character Map Select bits that form Character Set A Select
/// (bit 5 is field bit 2, bits 3:2 are field bits 1:0).
///
/// Spec: FreeVGA Sequencer Character Map Select Register (index `03h`).
pub const VGA_SEQ_CHAR_MAP_A_MASK: u8 = 0b0010_1100;
/// Sequencer Character Map Select bits that form Character Set B Select
/// (bit 4 is field bit 2, bits 1:0 are field bits 1:0).
///
/// Spec: FreeVGA Sequencer Character Map Select Register (index `03h`).
pub const VGA_SEQ_CHAR_MAP_B_MASK: u8 = 0b0001_0011;

/// Text attribute bit 3 — foreground intensity *and* character-set select.
///
/// Spec: FreeVGA Fonts — "If bit 3 of a character's attribute byte is set to 1,
/// then the character set selected by Character Set A Select field, otherwise
/// the character set specified by Character Set B Select field is used."
pub const VGA_TEXT_ATTR_FONT_SELECT: u8 = 0x08;

/// Attribute Controller Mode Control bit0 — Attribute Controller Graphics
/// Enable. Spec: FreeVGA Attribute Mode Control Register (index `10h`) `ATGE`.
pub const VGA_ATC_MODE_ATGE: u8 = 0x01;
/// Attribute Controller Mode Control bit2 — Line Graphics Enable.
///
/// Spec: IBM PS/2 Video Subsystems Figure 2-79 Mode Control — the ninth dot of
/// character codes `C0h`–`DFh` is made identical to the eighth when this bit is
/// set. FreeVGA's `LGA` prose states the inverse polarity; see
/// `docs/vga-r3-character-generator.md`.
pub const VGA_ATC_MODE_LINE_GRAPHICS: u8 = 0x04;
const _: () = assert!(VGA_ATC_MODE_CONTROL_DEFAULT & VGA_ATC_MODE_LINE_GRAPHICS != 0);

/// CRTC Mode Control register index.
///
/// Spec: FreeVGA CRT Controller Registers — index `17h`.
pub const VGA_CRTC_MODE_CONTROL: u8 = 0x17;
/// CRTC Mode Control bit6 — Word/Byte Mode Select (set selects byte mode).
///
/// Spec: FreeVGA CRTC Mode Control — "When this bit is set to 0, the word mode
/// is selected. The word mode shifts the memory-address counter bits to the
/// left by one bit ... When set to 1, bit 6 selects the byte address mode."
pub const VGA_CRTC_MODE_BYTE_ADDRESSING: u8 = 0x40;

/// Text cell width in dots when Sequencer Clocking Mode selects 9 dots.
///
/// Spec: FreeVGA Clocking Mode Register (index `01h`) bit0 — "0 - Selects 9
/// dots per character. 1 - Selects 8 dots per character."
pub const VGA_TEXT_CELL_WIDTH_9DOT: usize = 9;
/// Text cell width in dots when Sequencer Clocking Mode selects 8 dots.
pub const VGA_TEXT_CELL_WIDTH_8DOT: usize = 8;

/// First character code whose ninth dot replicates the eighth under Line
/// Graphics Enable. Spec: IBM PS/2 Video Subsystems Figure 2-79; FreeVGA Fonts.
pub const VGA_LINE_GRAPHICS_FIRST_CODE: u8 = 0xC0;
/// Last character code whose ninth dot replicates the eighth under Line
/// Graphics Enable.
pub const VGA_LINE_GRAPHICS_LAST_CODE: u8 = 0xDF;

/// Attribute foreground bits (2:0) that select underline in text mode.
///
/// Spec: FreeVGA "VGA Text Mode Operation", Attributes — "If bits 2-0 of the
/// attribute byte is equal to 001b and bits 6-4 of the attribute byte is equal
/// to 000b, then the line of the character specified by the Underline Location
/// field is replaced with the foreground color."
pub const VGA_TEXT_UNDERLINE_FG_BITS: u8 = 0x07;
/// Attribute foreground pattern that selects underline.
pub const VGA_TEXT_UNDERLINE_FG_VALUE: u8 = 0x01;
/// Attribute background bits (6:4) that must be zero for underline.
pub const VGA_TEXT_UNDERLINE_BG_BITS: u8 = 0x70;

/// Attribute Controller Mode Control bit6 — 8-bit Color Enable.
///
/// Spec: FreeVGA Attribute Mode Control Register — "When this bit is set to 1,
/// the video data is sampled so that eight bits are available to select a color
/// in the 256-color mode (0x13). This bit is set to 0 in all other modes."
pub const VGA_ATC_MODE_8BIT: u8 = 0x40;

/// Graphics Mode (`0x05`) bit6 — 256-Color Mode / Shift 256.
///
/// Spec: FreeVGA Graphics Mode Register `C256`; IBM PS/2 Video Subsystems
/// Figure 2-72 "256-Color Shift Mode".
pub const VGA_GC_MODE_SHIFT256: u8 = 0x40;

/// Displayed width of the chain-4 256-color graphics fetch, in pixels.
///
/// Spec: IBM VGA mode 13h is 320×200 with 256 colors.
pub const VGA_MODE13_WIDTH: usize = 320;
/// Displayed height of the chain-4 256-color graphics fetch, in rows.
pub const VGA_MODE13_HEIGHT: usize = 200;

/// Display mode the renderer can currently produce.
///
/// This model renders exactly two programmings and says so; it is not general
/// VGA mode coverage. In particular there is **no** planar 16-color renderer
/// (modes 0Dh/0Eh/10h/12h) and no VBE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VgaRenderMode {
    /// Alphanumeric (character generator) fetch: codes from map 0, attributes
    /// from map 1, glyphs from map 2.
    Text,
    /// Chain-4 256-color linear graphics — the mode-13h programming. Each
    /// display byte is one pixel and goes to the DAC through the PEL Mask.
    Graphics256Chain4,
    /// Programming the renderer does not model. No frame is produced.
    Unsupported,
}

/// One rendered frame as DAC indices, one byte per pixel, row-major.
///
/// The indices have already passed the display path this model implements —
/// for text, ATC Internal Palette → Color Select → PEL Mask. A host converts
/// them to colors through the DAC with [`VgaText::frame_rgba8`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VgaFrame {
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
    /// `width * height` DAC indices, row-major, top-left origin.
    pub pixels: Vec<u8>,
    /// Which display fetch produced this frame.
    pub mode: VgaRenderMode,
}

impl VgaFrame {
    /// DAC index at `(x, y)`, or `None` outside the frame.
    pub fn index_at(&self, x: usize, y: usize) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels.get(y * self.width + x).copied()
    }

    /// One row of DAC indices, or `None` outside the frame.
    pub fn row(&self, y: usize) -> Option<&[u8]> {
        if y >= self.height {
            return None;
        }
        self.pixels.get(y * self.width..(y + 1) * self.width)
    }
}

/// Color text-mode frame buffer + CRTC + Sequencer + GC + ATC + Misc + DAC stubs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VgaText {
    /// Latched CRTC index (written via active IOAS CRTC index port).
    pub crtc_index: u8,
    /// CRTC register file (store/readback; cursor indexes have helpers;
    /// Protect at `0x11` bit7 gates writes to `0x00`–`0x07`).
    pub crtc_regs: [u8; VGA_CRTC_REG_COUNT],
    /// Latched Sequencer index (written via `0x3C4`).
    pub seq_index: u8,
    /// Sequencer register file (noop store/readback).
    pub seq_regs: [u8; VGA_SEQ_REG_COUNT],
    /// Latched Graphics Controller index (written via `0x3CE`).
    pub gc_index: u8,
    /// Graphics Controller register file (noop store/readback).
    pub gc_regs: [u8; VGA_GC_REG_COUNT],
    /// Display memory: [`VGA_PLANE_COUNT`] × [`VGA_PLANE_SIZE`] bytes,
    /// map-major (map `p` offset `o` at `p * VGA_PLANE_SIZE + o`).
    ///
    /// This is the **only** display memory. Guest accesses reach it through
    /// [`VgaText::mmio_read_u8`] / [`VgaText::mmio_write_u8`] and the Graphics
    /// Controller data path; host helpers (`read_u8` / `write_u8` /
    /// `char_at` / `attr_at` / `put_char`) address the alphanumeric
    /// character/attribute interleave over the same bytes; the character
    /// generator fetches from it.
    pub planes: Vec<u8>,
    /// Graphics Controller data latches, one per map.
    ///
    /// Spec: IBM PS/2 Video Subsystems §2 "Graphics Controller" / OSDev VGA
    /// Hardware "The Latches" — a system read loads all four.
    pub gc_latches: [u8; VGA_PLANE_COUNT],
    /// Attribute Address register (bits 4:0 index, bit 5 PAS).
    pub atc_index: u8,
    /// Attribute Controller register file (noop store/readback).
    pub atc_regs: [u8; VGA_ATC_REG_COUNT],
    /// ATC flip-flop: `false` = next `0x3C0` write is address; `true` = data.
    pub atc_flip_flop_data: bool,
    /// Deterministic Input Status #1 phase (advances on each active-map status
    /// read — `0x3DA` or `0x3BA` per Misc IOAS).
    ///
    /// See [`VGA_STATUS1_PHASE_PERIOD`] for the active/retrace model.
    pub status1_phase: u8,
    /// Miscellaneous Output Register (store via `0x3C2`, read via `0x3CC`).
    ///
    /// Bit0 ([`VGA_MISC_IOAS`]) selects CRTC / Input Status #1 port ownership.
    /// Bit1 ([`VGA_MISC_RAM_ENABLE`]) gates CPU text-plane `read_u8`/`write_u8`.
    pub misc_output: u8,
    /// PEL Mask (`0x3C6`): display-path color-index AND (default
    /// [`VGA_DAC_PEL_MASK_DEFAULT`]). Applied by host text attr→DAC helpers;
    /// not applied to [`VGA_DAC_DATA`] R/W.
    pub dac_pel_mask: u8,
    /// DAC color RAM: 256 entries × RGB (6-bit components stored).
    pub dac_ram: [[u8; 3]; VGA_DAC_ENTRY_COUNT],
    /// Current DAC write index (set via `0x3C8`).
    pub dac_write_index: u8,
    /// Current DAC read index (set via write to `0x3C7`).
    pub dac_read_index: u8,
    /// Write channel: `0`=R, `1`=G, `2`=B.
    pub dac_write_channel: u8,
    /// Read channel: `0`=R, `1`=G, `2`=B.
    pub dac_read_channel: u8,
    /// DAC State bits 1:0 ([`VGA_DAC_STATE_READ`] / [`VGA_DAC_STATE_WRITE`]).
    pub dac_state: u8,
}

impl Default for VgaText {
    fn default() -> Self {
        Self::new()
    }
}

impl VgaText {
    pub fn new() -> Self {
        let mut v = Self {
            crtc_index: 0,
            crtc_regs: [0; VGA_CRTC_REG_COUNT],
            seq_index: 0,
            seq_regs: VGA_SEQ_DEFAULTS,
            gc_index: 0,
            gc_regs: VGA_GC_DEFAULTS,
            planes: vec![0; VGA_PLANE_COUNT * VGA_PLANE_SIZE],
            gc_latches: [0; VGA_PLANE_COUNT],
            atc_index: VGA_ATC_INDEX_DEFAULT,
            atc_regs: VGA_ATC_DEFAULTS,
            atc_flip_flop_data: false,
            status1_phase: 0,
            misc_output: VGA_MISC_OUTPUT_DEFAULT,
            dac_pel_mask: VGA_DAC_PEL_MASK_DEFAULT,
            dac_ram: vga_dac_default_ram(),
            dac_write_index: 0,
            dac_read_index: 0,
            dac_write_channel: 0,
            dac_read_channel: 0,
            dac_state: VGA_DAC_STATE_WRITE,
        };
        v.reset();
        v
    }

    /// Reset text plane: 80×25 → space/`0x07`; remainder cleared; CRTC cleared
    /// except Start Address High/Low [`VGA_CRTC_START_ADDR_HIGH`]/
    /// [`VGA_CRTC_START_ADDR_LOW`] = mode-03h defaults `0x00`/`0x00`,
    /// Maximum Scan Line [`VGA_CRTC_MAX_SCAN_LINE`] =
    /// [`VGA_CRTC_MAX_SCAN_LINE_DEFAULT`], Offset [`VGA_CRTC_OFFSET`] =
    /// [`VGA_CRTC_OFFSET_DEFAULT`], and Underline Location
    /// [`VGA_CRTC_UNDERLINE_LOCATION`] =
    /// [`VGA_CRTC_UNDERLINE_LOCATION_DEFAULT`]; Sequencer restored to
    /// [`VGA_SEQ_DEFAULTS`]; Graphics Controller restored to [`VGA_GC_DEFAULTS`];
    /// Attribute Controller restored to [`VGA_ATC_DEFAULTS`] with flip-flop in
    /// address state; Input Status #1 phase cleared; Misc Output restored to
    /// [`VGA_MISC_OUTPUT_DEFAULT`]; PEL Mask restored to
    /// [`VGA_DAC_PEL_MASK_DEFAULT`]; DAC RAM restored to mode-03h-ish defaults
    /// ([`vga_dac_default_ram`]).
    ///
    /// Spec: IBM VGA text — cells are (char, attr) pairs starting at `0xB8000`.
    /// The pair lives in display memory as map 0 and map 1 at the *same* even
    /// offset, which is where odd/even CPU addressing and word-mode display
    /// addressing both put it. **Model choice, not hardware:** real display
    /// memory holds whatever was there at power-on; this fill keeps the
    /// long-standing 80×25 blank screen the HELLO ROM path and the host text
    /// helpers expect. No font is installed in map 2.
    pub fn reset(&mut self) {
        self.crtc_index = 0;
        self.crtc_regs = [0; VGA_CRTC_REG_COUNT];
        self.crtc_regs[usize::from(VGA_CRTC_START_ADDR_HIGH)] = VGA_CRTC_START_ADDR_HIGH_DEFAULT;
        self.crtc_regs[usize::from(VGA_CRTC_START_ADDR_LOW)] = VGA_CRTC_START_ADDR_LOW_DEFAULT;
        self.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)] = VGA_CRTC_MAX_SCAN_LINE_DEFAULT;
        self.crtc_regs[usize::from(VGA_CRTC_OFFSET)] = VGA_CRTC_OFFSET_DEFAULT;
        self.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)] =
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT;
        self.seq_index = 0;
        self.seq_regs = VGA_SEQ_DEFAULTS;
        self.gc_index = 0;
        self.gc_regs = VGA_GC_DEFAULTS;
        self.planes.fill(0);
        for cell in 0..(VGA_TEXT_COLS * VGA_TEXT_ROWS) {
            let offset = cell * VGA_CELL_BYTES;
            self.planes[VGA_TEXT_CHAR_PLANE * VGA_PLANE_SIZE + offset] = VGA_DEFAULT_CHAR;
            self.planes[VGA_TEXT_ATTR_PLANE * VGA_PLANE_SIZE + offset] = VGA_DEFAULT_ATTR;
        }
        self.gc_latches = [0; VGA_PLANE_COUNT];
        self.atc_index = VGA_ATC_INDEX_DEFAULT;
        self.atc_regs = VGA_ATC_DEFAULTS;
        self.atc_flip_flop_data = false;
        self.status1_phase = 0;
        self.misc_output = VGA_MISC_OUTPUT_DEFAULT;
        self.dac_pel_mask = VGA_DAC_PEL_MASK_DEFAULT;
        self.dac_ram = vga_dac_default_ram();
        self.dac_write_index = 0;
        self.dac_read_index = 0;
        self.dac_write_channel = 0;
        self.dac_read_channel = 0;
        self.dac_state = VGA_DAC_STATE_WRITE;
    }

    /// True if `addr` (after A20) falls in the text plane.
    pub fn owns_addr(addr: u64) -> bool {
        (VGA_TEXT_BASE..VGA_TEXT_END).contains(&addr)
    }

    /// True when Misc Output IOAS selects the color I/O map (bit0 = 1).
    ///
    /// Spec: FreeVGA / IBM VGA Misc Output — IOAS selects `0x3Dx` vs `0x3Bx`.
    pub fn misc_ioas_color(&self) -> bool {
        self.misc_output & VGA_MISC_IOAS != 0
    }

    /// True when Misc Output RAM Enable is set (bit1 = 1).
    ///
    /// Spec: FreeVGA External Registers / IBM VGA Misc Output — when clear,
    /// CPU accesses to video RAM are disabled. This stub gates
    /// [`Self::read_u8`] / [`Self::write_u8`] only (test helpers may still
    /// inspect the plane buffer).
    pub fn misc_ram_enable(&self) -> bool {
        self.misc_output & VGA_MISC_RAM_ENABLE != 0
    }

    /// Misc Output Clock Select field (bits 3:2). Spec: FreeVGA / IBM VGA.
    pub fn misc_clock_select(&self) -> u8 {
        self.misc_output & VGA_MISC_CLOCK_SELECT
    }

    /// Misc Output HSYNC polarity bit (bit6). Spec: FreeVGA / IBM VGA.
    pub fn misc_hsync_polarity(&self) -> bool {
        self.misc_output & VGA_MISC_HSYNC_POLARITY != 0
    }

    /// Misc Output VSYNC polarity bit (bit7). Spec: FreeVGA / IBM VGA.
    pub fn misc_vsync_polarity(&self) -> bool {
        self.misc_output & VGA_MISC_VSYNC_POLARITY != 0
    }

    /// Sequencer Map Mask (`0x02`) map-enable bits 3:0.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-29 — M3E–M0E enable system
    /// access to the corresponding map.
    pub fn seq_map_mask(&self) -> u8 {
        self.seq_regs[usize::from(VGA_SEQ_MAP_MASK)] & VGA_SEQ_MAP_MASK_PLANES
    }

    /// True when Memory Mode Chain 4 (bit3) is set.
    pub fn seq_chain4_enabled(&self) -> bool {
        self.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)] & VGA_SEQ_MEMORY_MODE_CHAIN4 != 0
    }

    /// True when odd/even host addressing is active (Memory Mode bit2 clear).
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 — OE = 0 sends even system
    /// addresses to maps 0 and 2 and odd addresses to maps 1 and 3.
    pub fn seq_odd_even_enabled(&self) -> bool {
        self.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)] & VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE == 0
    }

    /// True when Memory Mode Extended Memory (bit1) is set (256 KB enabled).
    pub fn seq_extended_memory(&self) -> bool {
        self.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)] & VGA_SEQ_MEMORY_MODE_EXTENDED != 0
    }

    /// Addressable bytes per map for the current Extended Memory setting.
    pub fn plane_size_bytes(&self) -> usize {
        if self.seq_extended_memory() {
            VGA_PLANE_SIZE
        } else {
            VGA_PLANE_SIZE_NO_EXTENDED
        }
    }

    /// Graphics Controller Miscellaneous Memory Map Select field (bits 3:2).
    pub fn gc_memory_map_select(&self) -> u8 {
        (self.gc_regs[usize::from(VGA_GC_MISC)] & VGA_GC_MISC_MEMORY_MAP_MASK)
            >> VGA_GC_MISC_MEMORY_MAP_SHIFT
    }

    /// True when Graphics Controller Miscellaneous Chain Odd/Even (bit1) is set.
    pub fn gc_chain_odd_even(&self) -> bool {
        self.gc_regs[usize::from(VGA_GC_MISC)] & VGA_GC_MISC_CHAIN_ODD_EVEN != 0
    }

    /// True when Graphics Controller Miscellaneous selects graphics mode (bit0).
    pub fn gc_graphics_mode(&self) -> bool {
        self.gc_regs[usize::from(VGA_GC_MISC)] & VGA_GC_MISC_GRAPHICS_MODE != 0
    }

    /// CPU display window currently decoded (`base..end`).
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-75 Video Memory Assignments —
    /// Memory Map Select `00` = `A0000` for 128 KB, `01` = `A0000` for 64 KB,
    /// `10` = `B0000` for 32 KB, `11` = `B8000` for 32 KB.
    pub fn display_window(&self) -> (u64, u64) {
        match self.gc_memory_map_select() {
            VGA_GC_MEMORY_MAP_A0000_128K => (VGA_WINDOW_A0000_BASE, VGA_TEXT_END),
            VGA_GC_MEMORY_MAP_A0000_64K => (VGA_WINDOW_A0000_BASE, VGA_WINDOW_B0000_BASE),
            VGA_GC_MEMORY_MAP_B0000_32K => (VGA_WINDOW_B0000_BASE, VGA_TEXT_BASE),
            _ => (VGA_TEXT_BASE, VGA_TEXT_END),
        }
    }

    /// True when the video subsystem currently claims CPU accesses to `addr`.
    ///
    /// Requires Misc Output RAM Enable and membership of the Memory Map Select
    /// window. Spec: FreeVGA / IBM Misc Output bit1 + IBM Figure 2-75.
    pub fn owns_display_addr(&self, addr: u64) -> bool {
        let (base, end) = self.display_window();
        self.misc_ram_enable() && (base..end).contains(&addr)
    }

    /// Addressing model currently programmed.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figures 2-33 / 2-34 — Chain 4 takes
    /// precedence over odd/even (it replaces map selection entirely with
    /// A1/A0); otherwise Memory Mode OE = 0 gives odd/even and OE = 1 gives
    /// planar Map-Mask-only addressing. Graphics Controller Miscellaneous
    /// Chain Odd/Even (Figure 2-74 OE) is a second, independent source of
    /// odd/even host addressing, so either bit selects it.
    pub fn plane_addressing(&self) -> VgaPlaneAddressing {
        if self.seq_chain4_enabled() {
            VgaPlaneAddressing::Chain4
        } else if self.seq_odd_even_enabled() || self.gc_chain_odd_even() {
            VgaPlaneAddressing::OddEven
        } else {
            VgaPlaneAddressing::Planar
        }
    }

    /// Decode a CPU display-window address into map targets plus map offset.
    ///
    /// Returns `None` when `addr` is outside [`Self::display_window`].
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-34 (Chain 4: A1 A0 select the
    /// map) and Figure 2-33 (odd/even: even addresses → maps 0+2, odd → maps
    /// 1+3). Per-map offsets follow the officially documented, hardware-observed
    /// forms recorded in OSDev VGA Hardware "Addressing Logic": chain-4 keeps
    /// the host address with A1:A0 cleared, odd/even keeps it with A0 cleared,
    /// and planar mode passes it through. QEMU's alternative chain-4 offset
    /// (`addr >> 2`) is **not** modeled.
    pub fn plane_access(&self, addr: u64) -> Option<VgaPlaneAccess> {
        let (base, end) = self.display_window();
        if !(base..end).contains(&addr) {
            return None;
        }
        let window_offset = (addr - base) as usize;
        let addressing = self.plane_addressing();
        let (planes, raw_offset) = match addressing {
            VgaPlaneAddressing::Chain4 => (1u8 << (window_offset & 0b11), window_offset & !0b11),
            VgaPlaneAddressing::OddEven => {
                // Even → maps 0 and 2; odd → maps 1 and 3.
                let low = (window_offset & 1) as u8;
                ((0b0001 << low) | (0b0100 << low), window_offset & !1)
            }
            VgaPlaneAddressing::Planar => (VGA_SEQ_MAP_MASK_PLANES, window_offset),
        };
        let offset = raw_offset % self.plane_size_bytes();
        Some(VgaPlaneAccess {
            planes,
            write_planes: planes & self.seq_map_mask(),
            offset,
            addressing,
        })
    }

    /// Maps that a CPU write to `addr` would update (address decode AND Map Mask).
    pub fn plane_write_mask(&self, addr: u64) -> u8 {
        self.plane_access(addr)
            .map(|access| access.write_planes)
            .unwrap_or(0)
    }

    /// Per-map byte offset a CPU access to `addr` resolves to.
    pub fn plane_offset(&self, addr: u64) -> Option<usize> {
        self.plane_access(addr).map(|access| access.offset)
    }

    /// Read one byte of display memory directly (host/test helper, no GC path).
    pub fn plane_byte(&self, plane: usize, offset: usize) -> Option<u8> {
        if plane >= VGA_PLANE_COUNT || offset >= VGA_PLANE_SIZE {
            return None;
        }
        Some(self.planes[plane * VGA_PLANE_SIZE + offset])
    }

    /// Write one byte of display memory directly (host/test helper, no GC path).
    pub fn set_plane_byte(&mut self, plane: usize, offset: usize, value: u8) -> bool {
        if plane >= VGA_PLANE_COUNT || offset >= VGA_PLANE_SIZE {
            return false;
        }
        self.planes[plane * VGA_PLANE_SIZE + offset] = value;
        true
    }

    /// Graphics Mode Write Mode field (bits 1:0). Spec: IBM Figure 2-72.
    pub fn gc_write_mode(&self) -> u8 {
        self.gc_regs[usize::from(VGA_GC_MODE)] & VGA_GC_MODE_WRITE_MASK
    }

    /// Graphics Mode Read Mode bit (bit3). Spec: IBM Figure 2-72.
    pub fn gc_read_mode(&self) -> u8 {
        u8::from(self.gc_regs[usize::from(VGA_GC_MODE)] & VGA_GC_MODE_READ != 0)
    }

    /// Graphics Mode Host Odd/Even Memory Read Addressing bit (bit4).
    ///
    /// Spec: FreeVGA Graphics Registers, Graphics Mode "Host O/E".
    pub fn gc_host_odd_even_read(&self) -> bool {
        self.gc_regs[usize::from(VGA_GC_MODE)] & VGA_GC_MODE_HOST_ODD_EVEN_READ != 0
    }

    /// Data Rotate rotate count (bits 2:0). Spec: IBM Figure 2-69.
    pub fn gc_rotate_count(&self) -> u32 {
        u32::from(self.gc_regs[usize::from(VGA_GC_DATA_ROTATE)] & VGA_GC_ROTATE_COUNT_MASK)
    }

    /// Data Rotate Function Select (bits 4:3). Spec: IBM Figure 2-70.
    pub fn gc_function_select(&self) -> u8 {
        self.gc_regs[usize::from(VGA_GC_DATA_ROTATE)] & VGA_GC_FUNCTION_SELECT_MASK
    }

    /// Bit Mask register value. Spec: IBM Figure 2-77.
    pub fn gc_bit_mask(&self) -> u8 {
        self.gc_regs[usize::from(VGA_GC_BIT_MASK)]
    }

    /// Set/Reset map values (bits 3:0). Spec: IBM Figure 2-66.
    pub fn gc_set_reset(&self) -> u8 {
        self.gc_regs[usize::from(VGA_GC_SET_RESET)] & VGA_SEQ_MAP_MASK_PLANES
    }

    /// Enable Set/Reset map bits (bits 3:0). Spec: IBM Figure 2-67.
    pub fn gc_enable_set_reset(&self) -> u8 {
        self.gc_regs[usize::from(VGA_GC_ENABLE_SET_RESET)] & VGA_SEQ_MAP_MASK_PLANES
    }

    /// Read Map Select map number (bits 1:0). Spec: IBM Figure 2-71.
    pub fn gc_read_map_select(&self) -> usize {
        usize::from(self.gc_regs[usize::from(VGA_GC_READ_MAP_SELECT)] & VGA_GC_READ_MAP_SELECT_MASK)
    }

    /// Color Compare value (bits 3:0). Spec: IBM Figure 2-68.
    pub fn gc_color_compare(&self) -> u8 {
        self.gc_regs[usize::from(VGA_GC_COLOR_COMPARE)] & VGA_SEQ_MAP_MASK_PLANES
    }

    /// Color Don't Care participating maps (bits 3:0). Spec: IBM Figure 2-76.
    pub fn gc_color_dont_care(&self) -> u8 {
        self.gc_regs[usize::from(VGA_GC_COLOR_DONT_CARE)] & VGA_SEQ_MAP_MASK_PLANES
    }

    /// Expand one map bit to the 8-bit value written to that map.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-66 — "the set/reset bit, if
    /// enabled, is written to all 8 bits within that map".
    fn expand_map_bit(value: u8, plane: usize) -> u8 {
        if value & (1 << plane) != 0 {
            0xFF
        } else {
            0x00
        }
    }

    /// Apply Function Select between write data and the latched map byte.
    fn apply_function_select(&self, data: u8, latch: u8) -> u8 {
        match self.gc_function_select() {
            VGA_GC_FUNCTION_AND => data & latch,
            VGA_GC_FUNCTION_OR => data | latch,
            VGA_GC_FUNCTION_XOR => data ^ latch,
            _ => data,
        }
    }

    /// Blend an ALU result with the latched byte through a bit mask.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-77 — a clear mask bit keeps the
    /// latched bit for that position.
    fn blend_with_latch(result: u8, latch: u8, mask: u8) -> u8 {
        (result & mask) | (latch & !mask)
    }

    /// Read display memory through the Graphics Controller read path.
    ///
    /// Loads all four latches from the addressed map offset, then returns the
    /// read-mode result: read mode 0 returns the map named by Read Map Select
    /// (or by A1:A0 while Chain 4 is set), read mode 1 returns the color
    /// compare of the participating maps.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figures 2-68, 2-71, 2-72 (RM), 2-76;
    /// OSDev VGA Hardware "The Latches". Returns `None` when Misc Output RAM
    /// Enable is clear or the address is outside [`Self::display_window`].
    pub fn gc_read_u8(&mut self, addr: u64) -> Option<u8> {
        if !self.misc_ram_enable() {
            return None;
        }
        let access = self.plane_access(addr)?;
        for plane in 0..VGA_PLANE_COUNT {
            self.gc_latches[plane] = self.planes[plane * VGA_PLANE_SIZE + access.offset];
        }
        if self.gc_read_mode() == 1 {
            let compare = self.gc_color_compare();
            let participating = self.gc_color_dont_care();
            let mut result = 0u8;
            for bit in 0..8 {
                let mut matches = true;
                for plane in 0..VGA_PLANE_COUNT {
                    if participating & (1 << plane) == 0 {
                        continue;
                    }
                    let map_bit = (self.gc_latches[plane] >> bit) & 1;
                    let want = (compare >> plane) & 1;
                    if map_bit != want {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    result |= 1 << bit;
                }
            }
            return Some(result);
        }
        let plane = if access.addressing == VgaPlaneAddressing::Chain4 {
            access.planes.trailing_zeros() as usize
        } else if self.gc_host_odd_even_read() {
            // Host Odd/Even read addressing: the host address bit A0 replaces
            // bit 0 of Read Map Select, so an even address reads the even map
            // of the pair and an odd address the odd map. A0 comes from the
            // host address, not from `access.offset`, because odd/even
            // addressing has already cleared it there.
            let host_a0 = usize::from((addr - self.display_window().0) & 1 != 0);
            (self.gc_read_map_select() & !1) | host_a0
        } else {
            self.gc_read_map_select()
        };
        Some(self.gc_latches[plane])
    }

    /// Write display memory through the Graphics Controller write path.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-73 Write Mode Definitions plus
    /// Figures 2-66/2-67 (Set/Reset), 2-69/2-70 (rotate + Function Select),
    /// 2-77 (Bit Mask) and Figure 2-29 (Map Mask); OSDev VGA Hardware
    /// "Read/Write logic" for the per-step ordering.
    ///
    /// Write mode 3 applies Function Select between the expanded Set/Reset
    /// value and the latch, like modes 0 and 2. Sources conflict here: OSDev's
    /// write-mode-3 step list omits the ALU stage, while Michael Abrash's
    /// *Graphics Programming Black Book* chapter 26 documents its write-mode-3
    /// helper as "Forces ALU function to 'move'" — a redundant step if the
    /// stage were bypassed — and the Graphics Controller data flow places one
    /// ALU between the Set/Reset multiplexer and the Bit Mask multiplexer for
    /// every write mode. This model follows Abrash. The default Function
    /// Select is replace, so ordinary drivers see no difference; see
    /// `docs/vga-r2-gc-datapath-fixes.md`.
    ///
    /// Returns `false` when Misc Output RAM Enable is clear or the address is
    /// outside [`Self::display_window`].
    pub fn gc_write_u8(&mut self, addr: u64, value: u8) -> bool {
        if !self.misc_ram_enable() {
            return false;
        }
        let Some(access) = self.plane_access(addr) else {
            return false;
        };
        let mode = self.gc_write_mode();
        let rotated = value.rotate_right(self.gc_rotate_count());
        let bit_mask = self.gc_bit_mask();
        let set_reset = self.gc_set_reset();
        let enable_set_reset = self.gc_enable_set_reset();

        let mut results = [0u8; VGA_PLANE_COUNT];
        for (plane, slot) in results.iter_mut().enumerate() {
            let latch = self.gc_latches[plane];
            *slot = match mode {
                // Write mode 0: rotated system data (or Set/Reset for enabled
                // maps) through Function Select, then the Bit Mask.
                0 => {
                    let source = if enable_set_reset & (1 << plane) != 0 {
                        Self::expand_map_bit(set_reset, plane)
                    } else {
                        rotated
                    };
                    let alu = self.apply_function_select(source, latch);
                    Self::blend_with_latch(alu, latch, bit_mask)
                }
                // Write mode 1: the map receives the latch unchanged.
                1 => latch,
                // Write mode 2: map n filled with data bit n, then Function
                // Select and the Bit Mask.
                2 => {
                    let source = Self::expand_map_bit(value, plane);
                    let alu = self.apply_function_select(source, latch);
                    Self::blend_with_latch(alu, latch, bit_mask)
                }
                // Write mode 3: Set/Reset value (Enable Set/Reset ignored)
                // through Function Select, then a mask of rotated data ANDed
                // with the Bit Mask.
                _ => {
                    let mask = rotated & bit_mask;
                    let alu =
                        self.apply_function_select(Self::expand_map_bit(set_reset, plane), latch);
                    Self::blend_with_latch(alu, latch, mask)
                }
            };
        }

        for (plane, result) in results.iter().enumerate() {
            if access.write_planes & (1 << plane) != 0 {
                self.planes[plane * VGA_PLANE_SIZE + access.offset] = *result;
            }
        }
        true
    }

    /// True if this device owns the I/O port (CRTC + Sequencer + GC + ATC +
    /// DAC PEL / PEL Mask + Input Status #1 at the IOAS-selected addresses + Misc).
    ///
    /// Spec: FreeVGA / IBM — Misc Output IOAS selects color (`0x3D4`/`0x3D5`,
    /// `0x3DA`) vs mono (`0x3B4`/`0x3B5`, `0x3BA`) for CRTC and Input Status #1.
    pub fn owns_port(&self, port: u16) -> bool {
        match port {
            VGA_CRTC_INDEX | VGA_CRTC_DATA | VGA_INPUT_STATUS_1 => self.misc_ioas_color(),
            VGA_CRTC_INDEX_MONO | VGA_CRTC_DATA_MONO | VGA_INPUT_STATUS_1_MONO => {
                !self.misc_ioas_color()
            }
            VGA_SEQ_INDEX
            | VGA_SEQ_DATA
            | VGA_GC_INDEX
            | VGA_GC_DATA
            | VGA_ATC_ADDRESS_DATA
            | VGA_ATC_DATA_READ
            | VGA_DAC_PEL_MASK
            | VGA_DAC_READ_INDEX
            | VGA_DAC_WRITE_INDEX
            | VGA_DAC_DATA
            | VGA_MISC_OUTPUT_WRITE
            | VGA_MISC_OUTPUT_READ => true,
            _ => false,
        }
    }

    /// Map and offset in display memory for a host alphanumeric access.
    ///
    /// This is the character/attribute interleave the character generator
    /// fetches in word mode: an even byte is a character code in map 0 and the
    /// odd byte beside it is that cell's attribute in map 1, both at the same
    /// even map offset. It is *not* the Graphics Controller path — it applies
    /// no read mode, no write mode, no Map Mask, and disturbs no latch, which
    /// is what a host/test caller needs.
    ///
    /// Claims require Misc Output RAM Enable and membership of the Memory Map
    /// Select window ([`Self::owns_display_addr`]); the alphanumeric helpers
    /// additionally only speak for the `0xB8000`–`0xBFFFF` range they always
    /// have. The offset is relative to the selected window base, like every
    /// other address decode in this device.
    ///
    /// Spec: FreeVGA "VGA Text Mode Operation" Display Memory Organization
    /// (map 0 characters, map 1 attributes) + IBM Figure 2-33 (odd/even map
    /// pairing) + IBM Figure 2-75 (Memory Map Select) + Misc Output bit1.
    fn text_view_target(&self, addr: u64) -> Option<(usize, usize)> {
        if !self.owns_display_addr(addr) || !Self::owns_addr(addr) {
            // Same `None` / `false` as out-of-window so `MachineBus` falls
            // through to open-bus / PhysMem.
            return None;
        }
        let (base, _) = self.display_window();
        let window_offset = (addr - base) as usize;
        let plane = if window_offset & 1 == 0 {
            VGA_TEXT_CHAR_PLANE
        } else {
            VGA_TEXT_ATTR_PLANE
        };
        Some((plane, (window_offset & !1) % self.plane_size_bytes()))
    }

    /// Host read of the alphanumeric character/attribute interleave.
    ///
    /// Reads the one display memory ([`Self::planes`]) without touching the
    /// Graphics Controller latches. A guest read goes through
    /// [`Self::mmio_read_u8`] instead.
    pub fn read_u8(&self, addr: u64) -> Option<u8> {
        let (plane, offset) = self.text_view_target(addr)?;
        Some(self.planes[plane * VGA_PLANE_SIZE + offset])
    }

    /// Host write into the alphanumeric character/attribute interleave.
    ///
    /// Writes the one display memory ([`Self::planes`]) directly: no write
    /// mode, no Set/Reset, no Bit Mask, no Map Mask. A guest write goes
    /// through [`Self::mmio_write_u8`] instead.
    pub fn write_u8(&mut self, addr: u64, val: u8) -> bool {
        let Some((plane, offset)) = self.text_view_target(addr) else {
            return false;
        };
        self.planes[plane * VGA_PLANE_SIZE + offset] = val;
        true
    }

    /// Widest CPU display aperture the video subsystem can ever decode.
    ///
    /// Returns `(base, end)` — `end` exclusive. A system bus should route this
    /// whole range to [`Self::mmio_read_u8`] / [`Self::mmio_write_u8`] once and
    /// let the device decide per access, because the claimed sub-range moves
    /// with Graphics Controller Miscellaneous Memory Map Select and
    /// Miscellaneous Output RAM Enable.
    pub fn aperture() -> (u64, u64) {
        (VGA_APERTURE_BASE, VGA_APERTURE_END)
    }

    /// True if `addr` falls inside [`Self::aperture`] regardless of programming.
    pub fn in_aperture(addr: u64) -> bool {
        (VGA_APERTURE_BASE..VGA_APERTURE_END).contains(&addr)
    }

    /// True when a guest access to `addr` is claimed with the current
    /// programming (RAM Enable set and `addr` inside the selected window).
    pub fn mmio_claims(&self, addr: u64) -> bool {
        self.owns_display_addr(addr)
    }

    /// Guest CPU read of display memory — the single entry point for a bus.
    ///
    /// Performs the whole CPU-side pipeline: Miscellaneous Output RAM Enable
    /// gating, Graphics Controller Miscellaneous window decode, Sequencer plane
    /// addressing, and the Graphics Controller read path (all four latches
    /// loaded, then Read Map Select / Chain-4 map selection in read mode 0 or a
    /// Color Compare result in read mode 1).
    ///
    /// Every claimed address takes that path, `0xB8000` included: there is one
    /// display memory. Under the mode-03h reset programming — odd/even
    /// addressing, Map Mask `0x03`, read mode 0 with Graphics Mode Host
    /// Odd/Even read addressing set — a text read still returns the character
    /// map at even addresses and the attribute map at odd ones, which is what
    /// the separate text buffer used to return. See
    /// `docs/vga-r3-unified-display-memory.md`.
    ///
    /// Takes `&mut self` because a read loads the latches.
    ///
    /// Returns `None` when the access is not claimed, so the bus can fall
    /// through to open bus / RAM.
    ///
    /// Spec: FreeVGA External Registers (Misc Output bit1); IBM PS/2 Video
    /// Subsystems Figures 2-74 / 2-75 (Miscellaneous, Memory Map Select),
    /// 2-33 / 2-34 (Memory Mode addressing), 2-71 / 2-72 (Read Map Select,
    /// Read Mode).
    pub fn mmio_read_u8(&mut self, addr: u64) -> Option<u8> {
        self.gc_read_u8(addr)
    }

    /// Guest CPU write to display memory — the single entry point for a bus.
    ///
    /// Mirrors [`Self::mmio_read_u8`]: RAM Enable gating, window decode, plane
    /// addressing, then the Graphics Controller write path (write modes 0–3
    /// with Set/Reset, Enable Set/Reset, Data Rotate + Function Select, Bit
    /// Mask, and Map Mask plane write enables) — for every claimed address,
    /// `0xB8000` included.
    ///
    /// Returns `false` when the access is not claimed.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-73 Write Mode Definitions plus
    /// Figures 2-66/2-67, 2-69/2-70, 2-77 and Figure 2-29 Map Mask.
    pub fn mmio_write_u8(&mut self, addr: u64, value: u8) -> bool {
        self.gc_write_u8(addr, value)
    }

    /// Map offset of a visible text cell, as the character generator fetches it.
    ///
    /// Spec: FreeVGA CRT Controller — Start Address is the character index of
    /// the first displayed cell; Offset is the logical line width in words, so
    /// the character pitch is `Offset * 2` ([`VgaText::text_row_pitch_chars`]).
    /// The resulting CRTC address counter goes through
    /// [`VgaText::display_map_offset`], so the host helpers and the renderer
    /// read exactly the same bytes.
    fn cell_map_offset(&self, row: usize, col: usize) -> Option<usize> {
        if row >= VGA_TEXT_ROWS || col >= VGA_TEXT_COLS {
            return None;
        }
        let counter =
            usize::from(self.text_start_address()) + row * self.text_row_pitch_chars() + col;
        Some(self.display_map_offset(counter))
    }

    /// Character code displayed at `(row, col)` — map 0 at the fetched offset.
    pub fn char_at(&self, row: usize, col: usize) -> Option<u8> {
        let offset = self.cell_map_offset(row, col)?;
        Some(self.planes[VGA_TEXT_CHAR_PLANE * VGA_PLANE_SIZE + offset])
    }

    /// Attribute displayed at `(row, col)` — map 1 at the fetched offset.
    pub fn attr_at(&self, row: usize, col: usize) -> Option<u8> {
        let offset = self.cell_map_offset(row, col)?;
        Some(self.planes[VGA_TEXT_ATTR_PLANE * VGA_PLANE_SIZE + offset])
    }

    /// Host write of a character/attribute pair at `(row, col)`.
    pub fn put_char(&mut self, row: usize, col: usize, ch: u8, attr: u8) -> bool {
        let Some(offset) = self.cell_map_offset(row, col) else {
            return false;
        };
        self.planes[VGA_TEXT_CHAR_PLANE * VGA_PLANE_SIZE + offset] = ch;
        self.planes[VGA_TEXT_ATTR_PLANE * VGA_PLANE_SIZE + offset] = attr;
        true
    }

    /// 16-bit CRTC start address (`0x0C`:`0x0D`) — first displayed character.
    ///
    /// Spec: FreeVGA CRT Controller / IBM VGA — Start Address High/Low form a
    /// linear character offset into the refresh buffer (not a byte offset).
    /// Mode 03h defaults to `0`. Host text helpers apply this as the visible
    /// viewport origin; CPU `0xB8000` MMIO remains absolute.
    pub fn text_start_address(&self) -> u16 {
        let high = self.crtc_regs[usize::from(VGA_CRTC_START_ADDR_HIGH)];
        let low = self.crtc_regs[usize::from(VGA_CRTC_START_ADDR_LOW)];
        (u16::from(high) << 8) | u16::from(low)
    }

    /// Byte offset of the first displayed cell in the text plane (`start * 2`).
    ///
    /// Spec: IBM VGA / FreeVGA — each alphanumeric cell is two bytes
    /// (character + attribute). Used by host text viewport helpers.
    pub fn text_start_plane_offset(&self) -> usize {
        usize::from(self.text_start_address()) * VGA_CELL_BYTES
    }

    /// Logical text row pitch in character cells from CRTC Offset (`0x13`).
    ///
    /// Spec: FreeVGA CRT Controller — Offset is the logical line width in
    /// words (byte addressing). Each alphanumeric cell is one word (char +
    /// attribute), so character-cell pitch is `Offset * 2`. Mode-03h reset
    /// [`VGA_CRTC_OFFSET_DEFAULT`] (`0x28`) yields 80 cells — identity with
    /// [`VGA_TEXT_COLS`]. Host `char_at`/`attr_at`/`put_char` use this as the
    /// row stride; CPU `0xB8000` MMIO remains absolute.
    pub fn text_row_pitch_chars(&self) -> usize {
        usize::from(self.crtc_regs[usize::from(VGA_CRTC_OFFSET)]) * 2
    }

    /// Horizontal left-shift in pels from ATC Horizontal PEL Panning (`0x13`).
    ///
    /// Spec: FreeVGA Attribute Controller Registers — bits 3:0 are the Pixel
    /// Shift Count. For mode-03h-class 9-dot alphanumeric text, register value
    /// `8` (and `9`..=`15`) maps to 0 pels; values `0`..=`7` map to `n+1` pels
    /// (BIOS soft-scroll sequence `8,0,1,…,7` then bump CRTC start). Host
    /// `char_at`/`attr_at`/`put_char` remain on the character grid; this helper
    /// is the observable sub-cell offset for future canvas render.
    pub fn text_pel_pan(&self) -> u8 {
        match self.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)] & 0x0F {
            n @ 0..=7 => n + 1,
            _ => 0,
        }
    }

    /// Apply PEL Mask to a display-path DAC index before DAC RAM lookup.
    ///
    /// Spec: FreeVGA Color Registers / RBIL / IBM VGA — the PEL Mask (`0x3C6`)
    /// is ANDed with the color index of each displayed pixel before the DAC
    /// RAM is indexed. Default [`VGA_DAC_PEL_MASK_DEFAULT`] (`0xFF`) is identity.
    /// Does not alter [`VGA_DAC_DATA`] (`0x3C9`) programming. Host text helpers
    /// remap through [`Self::atc_palette_dac_index`] before calling this.
    pub fn display_dac_index(&self, color_index: u8) -> u8 {
        color_index & self.dac_pel_mask
    }

    /// Compose a 4-bit attribute color through ATC Internal Palette, Mode
    /// Control P54S, and Color Select into an 8-bit DAC index (before PEL Mask).
    ///
    /// Spec: FreeVGA Attribute Controller Registers — indexes `0x00`–`0x0F` are
    /// the Internal Palette. Color Select bits 3:2 supply DAC bits 7:6. With
    /// P54S clear, palette bits 5:0 supply DAC bits 5:0; with P54S set, Color
    /// Select bits 1:0 replace palette bits 5:4. PEL Mask is applied afterward
    /// by [`Self::display_dac_index`].
    pub fn atc_palette_dac_index(&self, color_index: u8) -> u8 {
        let palette =
            self.atc_regs[usize::from(color_index & VGA_ATC_PALETTE_LOW_MASK)] & VGA_DAC_COLOR_MASK;
        let color_select = self.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)];
        let bits_7_6 = (color_select & VGA_ATC_COLOR_SELECT_76_MASK) << 4;
        let bits_5_4 = if self.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_P54S != 0
        {
            (color_select & VGA_ATC_COLOR_SELECT_54_MASK) << 4
        } else {
            palette & !VGA_ATC_PALETTE_LOW_MASK
        };

        bits_7_6 | bits_5_4 | (palette & VGA_ATC_PALETTE_LOW_MASK)
    }

    /// Whether Attribute Controller Mode Control bit3 (BLINK) is set.
    ///
    /// Spec: FreeVGA Attribute Mode Control Register (index `0x10`) — `BLINK`.
    pub fn atc_blink_enabled(&self) -> bool {
        self.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_BLINK != 0
    }

    /// Whether this attribute cell blinks under current Mode Control.
    ///
    /// Spec: FreeVGA VGA Text Mode Operation — blink requires Mode Control
    /// BLINK set and attribute bit7 set.
    pub fn text_attr_blinks(&self, attr: u8) -> bool {
        self.atc_blink_enabled() && (attr & 0x80) != 0
    }

    /// Background attribute color index before Internal Palette / PEL Mask
    /// (Mode Control BLINK applied).
    ///
    /// Spec: FreeVGA Attribute Mode Control / VGA Text Mode Operation — when
    /// BLINK is set, attribute bit7 is blink enable and background uses bits
    /// 6:4 only; when clear, bits 7:4 form a 16-color background index.
    pub fn text_attr_bg_color_index(&self, attr: u8) -> u8 {
        let bg = attr >> 4;
        if self.atc_blink_enabled() {
            bg & 0x07
        } else {
            bg
        }
    }

    /// Foreground DAC index from a text attribute (Internal Palette + PEL Mask).
    ///
    /// Spec: FreeVGA Attribute Controller / IBM VGA / OSDev Text UI — attribute
    /// bits 3:0 select Internal Palette `0x00`–`0x0F`; the palette entry
    /// (bits 5:0) is the DAC index, then PEL Mask is applied.
    pub fn text_attr_fg_dac_index(&self, attr: u8) -> u8 {
        self.display_dac_index(self.atc_palette_dac_index(attr & 0x0F))
    }

    /// Background DAC index from a text attribute after Mode Control BLINK
    /// interpretation, Internal Palette remap, and PEL Mask.
    ///
    /// Spec: FreeVGA Attribute Mode Control / Internal Palette / IBM VGA —
    /// see [`Self::text_attr_bg_color_index`] then [`Self::atc_palette_dac_index`].
    pub fn text_attr_bg_dac_index(&self, attr: u8) -> u8 {
        self.display_dac_index(self.atc_palette_dac_index(self.text_attr_bg_color_index(attr)))
    }

    /// Foreground DAC index for a blink phase on the host text path.
    ///
    /// Spec: FreeVGA VGA Text Mode Operation — when blinking is enabled and
    /// attribute bit7 is set, the foreground alternates between the foreground
    /// and background colors. Pass `blink_off_half = true` for the invisible
    /// half (draw as background). Callers supply the phase; no VR/32 timer yet.
    pub fn text_attr_fg_dac_index_for_phase(&self, attr: u8, blink_off_half: bool) -> u8 {
        if blink_off_half && self.text_attr_blinks(attr) {
            self.text_attr_bg_dac_index(attr)
        } else {
            self.text_attr_fg_dac_index(attr)
        }
    }

    /// DAC RGB for a display-path color index (PEL Mask applied).
    ///
    /// Spec: FreeVGA — lookup uses `color_index & pel_mask`. Palette RAM at the
    /// unmasked index is unchanged.
    pub fn display_dac_rgb(&self, color_index: u8) -> [u8; 3] {
        self.dac_ram[usize::from(self.display_dac_index(color_index))]
    }

    /// 16-bit CRTC cursor character address (`0x0E`:`0x0F`).
    ///
    /// Spec: FreeVGA CRT Controller / IBM VGA — Cursor Location High/Low form a
    /// linear character offset into the refresh buffer (not a byte offset).
    pub fn crtc_cursor_location(&self) -> u16 {
        let high = self.crtc_regs[usize::from(VGA_CRTC_CURSOR_LOC_HIGH)];
        let low = self.crtc_regs[usize::from(VGA_CRTC_CURSOR_LOC_LOW)];
        (u16::from(high) << 8) | u16::from(low)
    }

    /// Byte offset of the cursor cell in the text plane (`location * 2`).
    ///
    /// Spec: IBM VGA / OSDev Text UI — each alphanumeric cell is two bytes
    /// (character + attribute).
    pub fn crtc_cursor_plane_offset(&self) -> usize {
        usize::from(self.crtc_cursor_location()) * VGA_CELL_BYTES
    }

    /// Text-mode `(row, col)` for an 80-column layout from cursor location.
    ///
    /// Spec: FreeVGA cursor location is a character index; classic mode 03h uses
    /// 80 columns. Does not subtract Start Address (`0x0C`/`0x0D`).
    pub fn crtc_cursor_row_col(&self) -> (usize, usize) {
        let loc = usize::from(self.crtc_cursor_location());
        (loc / VGA_TEXT_COLS, loc % VGA_TEXT_COLS)
    }

    /// Cursor Start scanline (CRTC `0x0A` bits 4:0).
    pub fn crtc_cursor_start_scanline(&self) -> u8 {
        self.crtc_regs[usize::from(VGA_CRTC_CURSOR_START)] & VGA_CRTC_CURSOR_SCANLINE_MASK
    }

    /// Cursor End scanline (CRTC `0x0B` bits 4:0).
    pub fn crtc_cursor_end_scanline(&self) -> u8 {
        self.crtc_regs[usize::from(VGA_CRTC_CURSOR_END)] & VGA_CRTC_CURSOR_SCANLINE_MASK
    }

    /// True when Cursor Start bit5 (Cursor Disable) is set.
    ///
    /// Spec: FreeVGA CRT Controller Registers — `CD` disables the hardware cursor.
    pub fn crtc_cursor_disabled(&self) -> bool {
        self.crtc_regs[usize::from(VGA_CRTC_CURSOR_START)] & VGA_CRTC_CURSOR_DISABLE != 0
    }

    /// True when Vertical Retrace End (`0x11`) Protect (bit7) is set.
    ///
    /// Spec: FreeVGA CRT Controller / IBM VGA — Protect ignores writes to
    /// indexes `0x00`–`0x07` except Overflow bit4.
    pub fn crtc_protect_enabled(&self) -> bool {
        self.crtc_regs[usize::from(VGA_CRTC_VERTICAL_RETRACE_END)] & VGA_CRTC_PROTECT != 0
    }

    fn crtc_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index);
        if i < VGA_CRTC_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    fn write_crtc_index(&mut self, value: u8) {
        self.crtc_index = value;
    }

    fn write_crtc_data(&mut self, value: u8) {
        // Spec: FreeVGA Vertical Retrace End Protect — when bit7 of index `0x11`
        // is set, indexes `0x00`–`0x07` ignore writes except Overflow bit4
        // (Line Compare bit8). Index `0x11` and indexes `>= 0x08` always write.
        if let Some(i) = Self::crtc_index_masked(self.crtc_index) {
            if self.crtc_protect_enabled() && i <= usize::from(VGA_CRTC_OVERFLOW) {
                if i == usize::from(VGA_CRTC_OVERFLOW) {
                    let old = self.crtc_regs[i];
                    self.crtc_regs[i] = (old & !VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8)
                        | (value & VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8);
                }
                return;
            }
            self.crtc_regs[i] = value;
        }
    }

    fn read_crtc_index(&self) -> u8 {
        self.crtc_index
    }

    fn read_crtc_data(&self) -> u8 {
        Self::crtc_index_masked(self.crtc_index)
            .map(|i| self.crtc_regs[i])
            .unwrap_or(0)
    }

    fn seq_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index);
        if i < VGA_SEQ_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    fn write_seq_index(&mut self, value: u8) {
        self.seq_index = value;
    }

    fn write_seq_data(&mut self, value: u8) {
        // Store only — map-mask / memory-mode / clocking bits are not enforced.
        if let Some(i) = Self::seq_index_masked(self.seq_index) {
            self.seq_regs[i] = value;
        }
    }

    fn read_seq_index(&self) -> u8 {
        self.seq_index
    }

    fn read_seq_data(&self) -> u8 {
        Self::seq_index_masked(self.seq_index)
            .map(|i| self.seq_regs[i])
            .unwrap_or(0)
    }

    fn gc_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index);
        if i < VGA_GC_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    fn write_gc_index(&mut self, value: u8) {
        self.gc_index = value;
    }

    fn write_gc_data(&mut self, value: u8) {
        // Store only — write-mode / map / bitmask bits are not enforced.
        if let Some(i) = Self::gc_index_masked(self.gc_index) {
            self.gc_regs[i] = value;
        }
    }

    fn read_gc_index(&self) -> u8 {
        self.gc_index
    }

    fn read_gc_data(&self) -> u8 {
        Self::gc_index_masked(self.gc_index)
            .map(|i| self.gc_regs[i])
            .unwrap_or(0)
    }

    fn write_misc_output(&mut self, value: u8) {
        // Store; IOAS (bit0) remaps CRTC index/data and Input Status #1
        // ownership. RAM Enable (bit1) gates CPU text-plane helpers.
        // Clock select / polarity bits are not enforced.
        self.misc_output = value;
    }

    fn read_misc_output(&self) -> u8 {
        self.misc_output
    }

    fn atc_index_masked(index: u8) -> Option<usize> {
        let i = usize::from(index & 0x1F);
        if i < VGA_ATC_REG_COUNT {
            Some(i)
        } else {
            None
        }
    }

    /// Reset ATC flip-flop so the next `0x3C0` write is an address byte.
    ///
    /// Spec: FreeVGA Accessing the Attribute Registers — read Input Status #1.
    fn reset_atc_flip_flop(&mut self) {
        self.atc_flip_flop_data = false;
    }

    fn write_atc_address_data(&mut self, value: u8) {
        if !self.atc_flip_flop_data {
            // Address write: bits 4:0 = index, bit 5 = PAS; flip to data.
            self.atc_index = value & 0x3F;
            self.atc_flip_flop_data = true;
        } else {
            // Data write for latched index; flip back to address.
            // Store only — palette / mode-control / plane bits are not enforced.
            if let Some(i) = Self::atc_index_masked(self.atc_index) {
                self.atc_regs[i] = value;
            }
            self.atc_flip_flop_data = false;
        }
    }

    fn read_atc_address(&self) -> u8 {
        // Spec: VGA — `0x3C0` read returns the Attribute Address register
        // (index + PAS); does not toggle the flip-flop.
        self.atc_index
    }

    fn read_atc_data(&self) -> u8 {
        // Spec: FreeVGA — `0x3C1` read returns selected data; no flip-flop toggle.
        Self::atc_index_masked(self.atc_index)
            .map(|i| self.atc_regs[i])
            .unwrap_or(0)
    }

    fn read_input_status_1(&mut self) -> u8 {
        // Spec: FreeVGA Accessing the Attribute Registers / External Registers —
        // read resets ATC flip-flop to address state and returns status bits.
        self.reset_atc_flip_flop();

        // Deterministic read-phase model (not CRTC-timed): even → display active
        // (DD=0, VR=0); odd → vertical retrace with display disabled (DD|VR).
        // Advances only here so firmware poll loops terminate without a tick.
        let phase = self.status1_phase;
        self.status1_phase = (self.status1_phase + 1) % VGA_STATUS1_PHASE_PERIOD;
        if phase.is_multiple_of(2) {
            0
        } else {
            VGA_STATUS1_DD | VGA_STATUS1_VR
        }
    }

    /// Spec: FreeVGA Color Registers — write `0x3C8` sets write index and arms
    /// the R→G→B write cycle; DAC state becomes write-ready.
    fn write_dac_write_index(&mut self, value: u8) {
        self.dac_write_index = value;
        self.dac_write_channel = 0;
        self.dac_state = VGA_DAC_STATE_WRITE;
    }

    /// Spec: FreeVGA Color Registers — write `0x3C7` sets read index and arms
    /// the R→G→B read cycle; DAC state becomes read-ready.
    fn write_dac_read_index(&mut self, value: u8) {
        self.dac_read_index = value;
        self.dac_read_channel = 0;
        self.dac_state = VGA_DAC_STATE_READ;
    }

    /// Spec: FreeVGA Color Registers — write `0x3C9` stores a 6-bit component
    /// (R then G then B); after blue the write index auto-increments.
    fn write_dac_data(&mut self, value: u8) {
        let ch = usize::from(self.dac_write_channel.min(2));
        let idx = usize::from(self.dac_write_index);
        self.dac_ram[idx][ch] = value & VGA_DAC_COLOR_MASK;
        self.dac_state = VGA_DAC_STATE_WRITE;
        if self.dac_write_channel >= 2 {
            self.dac_write_channel = 0;
            self.dac_write_index = self.dac_write_index.wrapping_add(1);
        } else {
            self.dac_write_channel += 1;
        }
    }

    /// Spec: FreeVGA Color Registers — read `0x3C9` returns a 6-bit component
    /// (R then G then B); after blue the read index auto-increments.
    fn read_dac_data(&mut self) -> u8 {
        let ch = usize::from(self.dac_read_channel.min(2));
        let idx = usize::from(self.dac_read_index);
        let value = self.dac_ram[idx][ch];
        self.dac_state = VGA_DAC_STATE_READ;
        if self.dac_read_channel >= 2 {
            self.dac_read_channel = 0;
            self.dac_read_index = self.dac_read_index.wrapping_add(1);
        } else {
            self.dac_read_channel += 1;
        }
        value
    }

    fn read_dac_write_index(&self) -> u8 {
        self.dac_write_index
    }

    fn read_dac_state(&self) -> u8 {
        self.dac_state
    }

    /// Spec: FreeVGA / RBIL — write `0x3C6` stores the PEL Mask (default `0xFF`).
    fn write_dac_pel_mask(&mut self, value: u8) {
        self.dac_pel_mask = value;
    }

    /// Spec: FreeVGA / RBIL — read `0x3C6` returns the current PEL Mask.
    fn read_dac_pel_mask(&self) -> u8 {
        self.dac_pel_mask
    }
}

/// Display fetch: the character generator and the frame it produces.
///
/// See `docs/vga-r3-character-generator.md` for the source list, the two
/// places where the sources conflict, and the model choices this section
/// makes where the hardware behavior depends on CRTC timing registers that
/// have no meaning in this model.
impl VgaText {
    /// Text cell width in dots from Sequencer Clocking Mode bit0.
    ///
    /// Spec: FreeVGA Clocking Mode Register (index `01h`) — bit0 clear selects
    /// 9 dots per character, set selects 8. The mode-03h reset default
    /// ([`VGA_SEQ_DEFAULTS`]) leaves it clear, so text cells are 9 dots wide.
    pub fn text_cell_width(&self) -> usize {
        if self.seq_regs[usize::from(VGA_SEQ_CLOCKING_MODE)] & VGA_SEQ_CLOCKING_8DOT != 0 {
            VGA_TEXT_CELL_WIDTH_8DOT
        } else {
            VGA_TEXT_CELL_WIDTH_9DOT
        }
    }

    /// Text cell height in scan lines from CRTC Maximum Scan Line (`0x09`).
    ///
    /// Spec: FreeVGA CRT Controller Maximum Scan Line Register — "In text
    /// modes, this field is programmed with the character height - 1". The
    /// mode-03h reset default `0x0F` gives 16 scan lines.
    pub fn text_cell_height(&self) -> usize {
        usize::from(self.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)] & VGA_CRTC_MAX_SCAN_MASK)
            + 1
    }

    /// True when CRTC Mode Control (`0x17`) bit6 selects byte addressing.
    ///
    /// Spec: FreeVGA CRTC Mode Control — bit6 clear is word mode, set is byte
    /// mode. The reset register file leaves it clear, which is also the
    /// mode-03h programming.
    pub fn crtc_byte_addressing(&self) -> bool {
        self.crtc_regs[usize::from(VGA_CRTC_MODE_CONTROL)] & VGA_CRTC_MODE_BYTE_ADDRESSING != 0
    }

    /// True when Underline Location (`0x14`) bit6 selects doubleword addressing.
    ///
    /// Spec: FreeVGA Underline Location Register — `DW`: "When this bit is set
    /// to 1, memory addresses are doubleword addresses."
    pub fn crtc_doubleword_addressing(&self) -> bool {
        self.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)] & VGA_CRTC_UNDERLINE_DW != 0
    }

    /// Multiplier applied to the CRTC address counter before it indexes a map.
    ///
    /// Spec: FreeVGA CRTC Mode Control (word/byte) and Underline Location
    /// (`DW`) — doubleword addressing shifts the counter left by two, word mode
    /// by one, byte mode not at all. This is the display-side counterpart of
    /// the CPU-side odd/even offset (`addr & !1`) recorded in
    /// `docs/vga-plane-memory-model.md`: in the mode-03h word-mode default the
    /// character at counter value *n* lives at map offset `2n`, exactly where
    /// an odd/even CPU write to `0xB8000 + 2n` puts it.
    ///
    /// The word-mode Address Wrap (`0x17` bit5) rotation of MA13/MA15 onto MA0
    /// is **not** modeled; this is a plain shift.
    pub fn crtc_address_multiplier(&self) -> usize {
        if self.crtc_doubleword_addressing() {
            4
        } else if self.crtc_byte_addressing() {
            1
        } else {
            2
        }
    }

    /// Map offset a display fetch reads for CRTC address-counter value `index`.
    ///
    /// The counter is 16 bits (FreeVGA Start Address Low: "this 16-bit field is
    /// sufficient to allow the screen to start at any memory address"), so it
    /// wraps at `0x10000`; the resulting map offset wraps inside the enabled
    /// map size ([`Self::plane_size_bytes`]).
    pub fn display_map_offset(&self, index: usize) -> usize {
        let counter = index & 0xFFFF;
        (counter * self.crtc_address_multiplier()) % self.plane_size_bytes()
    }

    /// Byte offset in map 2 of the font bank named by a 3-bit select field.
    ///
    /// Spec: FreeVGA Sequencer Character Map Select Register — the field is not
    /// contiguous for EGA compatibility: `000b` → `0000h`, `001b` → `4000h`,
    /// `010b` → `8000h`, `011b` → `C000h`, `100b` → `2000h`, `101b` → `6000h`,
    /// `110b` → `A000h`, `111b` → `E000h`.
    pub fn char_map_bank_offset(select: u8) -> usize {
        let select = usize::from(select & 0b111);
        ((select & 0b011) << 14) | ((select >> 2) << 13)
    }

    /// Character Set A Select field (Sequencer `0x03` bits 5, 3:2).
    ///
    /// Spec: FreeVGA Sequencer Character Map Select Register.
    pub fn seq_char_map_a_select(&self) -> u8 {
        let reg = self.seq_regs[usize::from(VGA_SEQ_CHAR_MAP_SELECT)] & VGA_SEQ_CHAR_MAP_A_MASK;
        ((reg & 0x20) >> 3) | ((reg & 0x0C) >> 2)
    }

    /// Character Set B Select field (Sequencer `0x03` bits 4, 1:0).
    ///
    /// Spec: FreeVGA Sequencer Character Map Select Register.
    pub fn seq_char_map_b_select(&self) -> u8 {
        let reg = self.seq_regs[usize::from(VGA_SEQ_CHAR_MAP_SELECT)] & VGA_SEQ_CHAR_MAP_B_MASK;
        ((reg & 0x10) >> 2) | (reg & 0x03)
    }

    /// Map-2 byte offset of the font bank used for a given attribute byte.
    ///
    /// Spec: FreeVGA Fonts — attribute bit 3 set selects Character Set A,
    /// clear selects Character Set B. Spec: FreeVGA Sequencer Memory Mode
    /// `Ext. Mem` — "This bit must be set to 1 to enable the character map
    /// selection described for the previous register", so with Extended Memory
    /// clear this model falls back to bank `0000h`.
    pub fn text_font_bank_offset(&self, attr: u8) -> usize {
        if !self.seq_extended_memory() {
            return 0;
        }
        let select = if attr & VGA_TEXT_ATTR_FONT_SELECT != 0 {
            self.seq_char_map_a_select()
        } else {
            self.seq_char_map_b_select()
        };
        Self::char_map_bank_offset(select)
    }

    /// One scan-line row of a glyph, as the character generator fetches it.
    ///
    /// `row` is the scan line within the cell. Rows at or beyond
    /// [`VGA_FONT_MAX_SCAN_LINES`] have no font byte and read as background.
    ///
    /// Spec: FreeVGA Fonts — "The offset in plane 2 of a character within a
    /// bank is determined by taking the character's value and multiplying it by
    /// 32. The first byte at this offset contains the 8 pixels of the top scan
    /// line".
    pub fn text_glyph_row(&self, code: u8, attr: u8, row: usize) -> u8 {
        if row >= VGA_FONT_MAX_SCAN_LINES {
            return 0;
        }
        let offset =
            (self.text_font_bank_offset(attr) + usize::from(code) * VGA_FONT_GLYPH_BYTES + row)
                % self.plane_size_bytes();
        self.planes[VGA_FONT_PLANE * VGA_PLANE_SIZE + offset]
    }

    /// True when Attribute Mode Control Line Graphics Enable is set.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-79 Mode Control bit2. Set means
    /// character codes `C0h`–`DFh` get a ninth dot identical to their eighth.
    pub fn atc_line_graphics_enabled(&self) -> bool {
        self.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_LINE_GRAPHICS != 0
    }

    /// True when Attribute Mode Control selects graphics (`ATGE`, bit0).
    ///
    /// Spec: FreeVGA Attribute Mode Control Register — "When set to 1, this bit
    /// selects the graphics mode of operation."
    pub fn atc_graphics_enabled(&self) -> bool {
        self.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_ATGE != 0
    }

    /// Scan line within a cell on which an underline is drawn.
    ///
    /// Spec: FreeVGA CRT Controller Underline Location Register bits 4:0. The
    /// mode-03h reset default `0x1F` puts it past a 16-line cell, which
    /// disables underlining — FreeVGA "VGA Text Mode Operation": "if the line
    /// specified by the Underline Location field is not normally displayed
    /// because it is greater than the maximum scan line ... then the underline
    /// capability is effectively disabled."
    pub fn crtc_underline_scanline(&self) -> u8 {
        self.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)] & VGA_CRTC_UNDERLINE_MASK
    }

    /// True when this attribute byte selects the underline attribute.
    ///
    /// Spec: FreeVGA "VGA Text Mode Operation", Attributes — attribute bits 2:0
    /// equal `001b` and bits 6:4 equal `000b`.
    pub fn text_attr_underlines(&self, attr: u8) -> bool {
        attr & VGA_TEXT_UNDERLINE_FG_BITS == VGA_TEXT_UNDERLINE_FG_VALUE
            && attr & VGA_TEXT_UNDERLINE_BG_BITS == 0
    }

    /// True when Attribute Mode Control 8-bit Color Enable is set.
    ///
    /// Spec: FreeVGA Attribute Mode Control Register `8BIT`.
    pub fn atc_8bit_color(&self) -> bool {
        self.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_8BIT != 0
    }

    /// True when Graphics Mode 256-Color Mode (`C256`) is set.
    ///
    /// Spec: FreeVGA Graphics Mode Register; IBM Figure 2-72.
    pub fn gc_shift256(&self) -> bool {
        self.gc_regs[usize::from(VGA_GC_MODE)] & VGA_GC_MODE_SHIFT256 != 0
    }

    /// True when the register file carries the whole mode-13h signature.
    ///
    /// All five must hold, because this model claims only this programming:
    /// Graphics Controller Miscellaneous Graphics/Alphanumeric (IBM Figure
    /// 2-74 bit0), Attribute Mode Control `ATGE` and `8BIT` (FreeVGA index
    /// `10h` bits 0 and 6), Sequencer Memory Mode Chain 4 (IBM Figure 2-34),
    /// and Graphics Mode `C256` (IBM Figure 2-72).
    ///
    /// The Memory Map Select CPU window is deliberately *not* part of the
    /// test: the CRTC addresses display memory directly, so where the CPU
    /// aperture sits does not change what is displayed.
    pub fn is_mode13h_programming(&self) -> bool {
        self.gc_graphics_mode()
            && self.atc_graphics_enabled()
            && self.atc_8bit_color()
            && self.seq_chain4_enabled()
            && self.gc_shift256()
    }

    /// Display fetch this model can produce with the current programming.
    ///
    /// Two fetches exist: the alphanumeric character generator and the
    /// chain-4 256-color linear fetch. Every other graphics programming —
    /// planar 16-color modes included — reports
    /// [`VgaRenderMode::Unsupported`] rather than rendering something that is
    /// not what the hardware would show.
    pub fn render_mode(&self) -> VgaRenderMode {
        if self.is_mode13h_programming() {
            return VgaRenderMode::Graphics256Chain4;
        }
        if self.gc_graphics_mode() || self.atc_graphics_enabled() {
            return VgaRenderMode::Unsupported;
        }
        VgaRenderMode::Text
    }

    /// Render the current display, or `None` when [`Self::render_mode`] reports
    /// a programming this model does not fetch.
    ///
    /// `blink_off_half` selects the invisible half of the blink cycle; the
    /// caller owns the phase because there is no vertical-retrace timer. It has
    /// no effect on a graphics fetch.
    pub fn render_frame(&self, blink_off_half: bool) -> Option<VgaFrame> {
        match self.render_mode() {
            VgaRenderMode::Text => Some(self.render_text_frame(blink_off_half)),
            VgaRenderMode::Graphics256Chain4 => Some(self.render_graphics256_frame()),
            VgaRenderMode::Unsupported => None,
        }
    }

    /// Render the alphanumeric display from plane memory.
    ///
    /// The fetch is the real hardware path: character codes from map 0,
    /// attributes from map 1, and glyph rows from the font bank in map 2 that
    /// Character Map Select and attribute bit 3 name. Every pixel is a DAC
    /// index that has already passed ATC Internal Palette → Color Select →
    /// PEL Mask.
    ///
    /// Spec: FreeVGA "VGA Text Mode Operation" (Display Memory Organization,
    /// Attributes, Fonts, Cursor); FreeVGA CRT Controller Start Address /
    /// Offset / Maximum Scan Line / Cursor Start / Cursor End / Cursor Location
    /// / Underline Location; FreeVGA Sequencer Clocking Mode and Character Map
    /// Select; IBM PS/2 Video Subsystems Figure 2-79 Mode Control.
    ///
    /// **Model choice:** the character grid is the fixed
    /// [`VGA_TEXT_COLS`]×[`VGA_TEXT_ROWS`] host grid the rest of this device
    /// uses. Horizontal and Vertical Display End are stored but do not size the
    /// frame, because this model has no CRTC timing and those registers have no
    /// reset defaults here. Maximum Scan Line bit7 (Scan Doubling), Preset Row
    /// Scan, Line Compare, Cursor Skew, Horizontal PEL Panning, Color Plane
    /// Enable, Overscan/border, and Screen Disable are not applied.
    pub fn render_text_frame(&self, blink_off_half: bool) -> VgaFrame {
        let cell_w = self.text_cell_width();
        let cell_h = self.text_cell_height();
        let width = VGA_TEXT_COLS * cell_w;
        let height = VGA_TEXT_ROWS * cell_h;
        let mut pixels = vec![0u8; width * height];

        let start = usize::from(self.text_start_address());
        let pitch = self.text_row_pitch_chars();
        let cursor_location = usize::from(self.crtc_cursor_location());
        let cursor_enabled = !self.crtc_cursor_disabled();
        let cursor_first = usize::from(self.crtc_cursor_start_scanline());
        let cursor_last = usize::from(self.crtc_cursor_end_scanline());
        let underline_row = usize::from(self.crtc_underline_scanline());
        let line_graphics = self.atc_line_graphics_enabled();

        for row in 0..VGA_TEXT_ROWS {
            for col in 0..VGA_TEXT_COLS {
                let counter = start + row * pitch + col;
                let offset = self.display_map_offset(counter);
                let code = self.planes[VGA_TEXT_CHAR_PLANE * VGA_PLANE_SIZE + offset];
                let attr = self.planes[VGA_TEXT_ATTR_PLANE * VGA_PLANE_SIZE + offset];

                let fg = self.text_attr_fg_dac_index_for_phase(attr, blink_off_half);
                let bg = self.text_attr_bg_dac_index(attr);
                let underlines = self.text_attr_underlines(attr);
                // Spec: FreeVGA Cursor Location Low — the hardware compares the
                // address of the character being displayed with the Cursor
                // Location field. The counter is 16 bits wide.
                let cursor_cell = cursor_enabled
                    && (counter & 0xFFFF) == cursor_location
                    && cursor_first <= cursor_last;
                let ninth_dot_repeats = line_graphics
                    && (VGA_LINE_GRAPHICS_FIRST_CODE..=VGA_LINE_GRAPHICS_LAST_CODE).contains(&code);

                for scan in 0..cell_h {
                    let glyph = self.text_glyph_row(code, attr, scan);
                    let solid = (underlines && scan == underline_row)
                        || (cursor_cell && (cursor_first..=cursor_last).contains(&scan));
                    let base = (row * cell_h + scan) * width + col * cell_w;
                    for dot in 0..cell_w {
                        let lit = if solid {
                            true
                        } else if dot < VGA_TEXT_CELL_WIDTH_8DOT {
                            glyph & (0x80 >> dot) != 0
                        } else {
                            // Spec: IBM Figure 2-79 / FreeVGA Fonts — the ninth
                            // dot is background unless Line Graphics Enable
                            // repeats the eighth for codes C0h-DFh.
                            ninth_dot_repeats && glyph & 0x01 != 0
                        };
                        pixels[base + dot] = if lit { fg } else { bg };
                    }
                }
            }
        }

        VgaFrame {
            width,
            height,
            pixels,
            mode: VgaRenderMode::Text,
        }
    }

    /// Byte distance between two displayed rows in a graphics fetch.
    ///
    /// Spec: FreeVGA CRTC Offset Register — "Beginning with the second scan
    /// line, the starting scan line is increased by twice the value of this
    /// register multiplied by the current memory address size". The mode-13h
    /// programming (Offset `0x28`, doubleword addressing) gives
    /// `0x28 * 2 * 4` = 320 bytes, one byte per pixel across the screen.
    pub fn graphics_row_stride_bytes(&self) -> usize {
        usize::from(self.crtc_regs[usize::from(VGA_CRTC_OFFSET)])
            * 2
            * self.crtc_address_multiplier()
    }

    /// DAC index of one chain-4 256-color pixel at display byte address
    /// `linear`.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-34 — with Chain 4 the two
    /// low-order address bits select the map, so display byte *n* lives in map
    /// `n & 3`. The per-map offset is the address with A1:A0 cleared, the same
    /// form the CPU side uses (`docs/vga-plane-memory-model.md`), so a byte the
    /// CPU wrote at `0xA0000 + n` is the pixel the display reads at *n*.
    ///
    /// Spec: FreeVGA Attribute Mode Control `8BIT` and Color Select — "In mode
    /// 13 hex, the 8-bit attribute is the digital color value to the video
    /// DAC", so the Internal Palette and Color Select take no part; only the
    /// PEL Mask is applied.
    pub fn graphics256_pixel_dac_index(&self, linear: usize) -> u8 {
        let map = linear & 0b11;
        let offset = (linear & !0b11) % self.plane_size_bytes();
        let pixel = self.planes[map * VGA_PLANE_SIZE + offset];
        self.display_dac_index(pixel)
    }

    /// Render the chain-4 256-color linear display (the mode-13h fetch).
    ///
    /// Row *r* starts at CRTC address counter `StartAddress + r * Offset * 2`,
    /// which the addressing multiplier turns into a byte address; pixel *x* of
    /// that row is the next byte along. Each byte is a DAC index after the PEL
    /// Mask.
    ///
    /// **Model choice:** the frame is a fixed
    /// [`VGA_MODE13_WIDTH`]×[`VGA_MODE13_HEIGHT`] window, for the same reason
    /// the text grid is fixed at 80×25 — this model has no CRTC timing, so
    /// Horizontal and Vertical Display End cannot size it. A wider Offset
    /// therefore behaves as a virtual resolution: the row stride grows while
    /// the visible window stays 320 pixels. Maximum Scan Line bit7 Scan
    /// Doubling (which real mode 13h uses to paint 200 rows on 400 scan lines)
    /// is not applied, so a row is one output row here.
    pub fn render_graphics256_frame(&self) -> VgaFrame {
        let width = VGA_MODE13_WIDTH;
        let height = VGA_MODE13_HEIGHT;
        let mut pixels = vec![0u8; width * height];
        let start_byte = usize::from(self.text_start_address()) * self.crtc_address_multiplier();
        let stride = self.graphics_row_stride_bytes();

        for row in 0..height {
            let row_byte = start_byte + row * stride;
            for x in 0..width {
                pixels[row * width + x] = self.graphics256_pixel_dac_index(row_byte + x);
            }
        }

        VgaFrame {
            width,
            height,
            pixels,
            mode: VgaRenderMode::Graphics256Chain4,
        }
    }

    /// DAC RAM entry for an already-masked display index (6-bit components).
    ///
    /// Spec: FreeVGA Color Registers — the DAC RAM stores 6-bit R, G and B.
    /// Unlike [`Self::display_dac_rgb`] this applies no PEL Mask, because a
    /// [`VgaFrame`] index has already passed it.
    pub fn dac_rgb6(&self, dac_index: u8) -> [u8; 3] {
        self.dac_ram[usize::from(dac_index)]
    }

    /// Expand a rendered frame to 8-bit RGBA for a host canvas or CLI.
    ///
    /// Four bytes per pixel in `R, G, B, 255` order, row-major. 6-bit DAC
    /// components are scaled to 8 bits by replicating the high bits
    /// (`v << 2 | v >> 4`), which maps `0x3F` to `0xFF` and `0x00` to `0x00`.
    pub fn frame_rgba8(&self, frame: &VgaFrame) -> Vec<u8> {
        let mut out = Vec::with_capacity(frame.pixels.len() * 4);
        for index in &frame.pixels {
            let [r, g, b] = self.dac_rgb6(*index);
            out.push((r << 2) | (r >> 4));
            out.push((g << 2) | (g >> 4));
            out.push((b << 2) | (b >> 4));
            out.push(0xFF);
        }
        out
    }
}

impl PortDevice for VgaText {
    fn port_read(&mut self, port: u16, _size: u8) -> u32 {
        match port {
            VGA_CRTC_INDEX if self.misc_ioas_color() => u32::from(self.read_crtc_index()),
            VGA_CRTC_DATA if self.misc_ioas_color() => u32::from(self.read_crtc_data()),
            VGA_CRTC_INDEX_MONO if !self.misc_ioas_color() => u32::from(self.read_crtc_index()),
            VGA_CRTC_DATA_MONO if !self.misc_ioas_color() => u32::from(self.read_crtc_data()),
            VGA_SEQ_INDEX => u32::from(self.read_seq_index()),
            VGA_SEQ_DATA => u32::from(self.read_seq_data()),
            VGA_GC_INDEX => u32::from(self.read_gc_index()),
            VGA_GC_DATA => u32::from(self.read_gc_data()),
            VGA_ATC_ADDRESS_DATA => u32::from(self.read_atc_address()),
            VGA_ATC_DATA_READ => u32::from(self.read_atc_data()),
            // Spec: FreeVGA / RBIL — read `0x3C6` = PEL Mask.
            VGA_DAC_PEL_MASK => u32::from(self.read_dac_pel_mask()),
            // Spec: FreeVGA Color Registers — read `0x3C7` = DAC State.
            VGA_DAC_READ_INDEX => u32::from(self.read_dac_state()),
            // Spec: FreeVGA Color Registers — read `0x3C8` = current write index.
            VGA_DAC_WRITE_INDEX => u32::from(self.read_dac_write_index()),
            VGA_DAC_DATA => u32::from(self.read_dac_data()),
            VGA_INPUT_STATUS_1 if self.misc_ioas_color() => u32::from(self.read_input_status_1()),
            VGA_INPUT_STATUS_1_MONO if !self.misc_ioas_color() => {
                u32::from(self.read_input_status_1())
            }
            // Spec: FreeVGA / OSDev — `0x3C2` is write-only; read is undefined.
            // Stub returns open-bus-style `0xFF` (use `0x3CC` for readback).
            VGA_MISC_OUTPUT_WRITE => 0xFF,
            VGA_MISC_OUTPUT_READ => u32::from(self.read_misc_output()),
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        match port {
            VGA_CRTC_INDEX if self.misc_ioas_color() => {
                // Spec: OSDev VGA Hardware — some guests write index+data as a
                // single word to 0x3D4 (low = index, high = data).
                if size >= 2 {
                    self.write_crtc_index(value as u8);
                    self.write_crtc_data((value >> 8) as u8);
                } else {
                    self.write_crtc_index(value as u8);
                }
            }
            VGA_CRTC_DATA if self.misc_ioas_color() => self.write_crtc_data(value as u8),
            VGA_CRTC_INDEX_MONO if !self.misc_ioas_color() => {
                // Spec: FreeVGA / IBM — mono map mirrors color word write at 0x3B4.
                if size >= 2 {
                    self.write_crtc_index(value as u8);
                    self.write_crtc_data((value >> 8) as u8);
                } else {
                    self.write_crtc_index(value as u8);
                }
            }
            VGA_CRTC_DATA_MONO if !self.misc_ioas_color() => self.write_crtc_data(value as u8),
            VGA_SEQ_INDEX => {
                // Mirror CRTC: 16-bit write to 0x3C4 (lo=index, hi=data).
                if size >= 2 {
                    self.write_seq_index(value as u8);
                    self.write_seq_data((value >> 8) as u8);
                } else {
                    self.write_seq_index(value as u8);
                }
            }
            VGA_SEQ_DATA => self.write_seq_data(value as u8),
            VGA_GC_INDEX => {
                // Mirror CRTC/Sequencer: 16-bit write to 0x3CE (lo=index, hi=data).
                if size >= 2 {
                    self.write_gc_index(value as u8);
                    self.write_gc_data((value >> 8) as u8);
                } else {
                    self.write_gc_index(value as u8);
                }
            }
            VGA_GC_DATA => self.write_gc_data(value as u8),
            VGA_ATC_ADDRESS_DATA => {
                // Spec: FreeVGA — consecutive writes to 0x3C0 are index then data.
                // A 16-bit OUT writes lo=index then hi=data through the flip-flop.
                if size >= 2 {
                    self.write_atc_address_data(value as u8);
                    self.write_atc_address_data((value >> 8) as u8);
                } else {
                    self.write_atc_address_data(value as u8);
                }
            }
            // Spec: FreeVGA — `0x3C1` is data-read; writes ignored.
            VGA_ATC_DATA_READ => {}
            // Spec: FreeVGA / RBIL — write `0x3C6` = PEL Mask.
            VGA_DAC_PEL_MASK => self.write_dac_pel_mask(value as u8),
            // Spec: FreeVGA Color Registers — write `0x3C7` = read-mode index.
            VGA_DAC_READ_INDEX => self.write_dac_read_index(value as u8),
            VGA_DAC_WRITE_INDEX => self.write_dac_write_index(value as u8),
            VGA_DAC_DATA => {
                // Spec: FreeVGA — RGB must be written as three successive bytes.
                // A 16-bit OUT supplies two of those bytes (lo then hi).
                if size >= 2 {
                    self.write_dac_data(value as u8);
                    self.write_dac_data((value >> 8) as u8);
                } else {
                    self.write_dac_data(value as u8);
                }
            }
            // Spec: FreeVGA — Input Status #1 is read-only; writes ignored.
            VGA_INPUT_STATUS_1 | VGA_INPUT_STATUS_1_MONO => {}
            VGA_MISC_OUTPUT_WRITE => self.write_misc_output(value as u8),
            // Spec: FreeVGA / OSDev — `0x3CC` is read-only; writes ignored.
            VGA_MISC_OUTPUT_READ => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_fills_80x25_space_attr07() {
        // Spec: IBM VGA text — default light-gray-on-black cells. The pair now
        // lives in the one display memory: character in map 0 and attribute in
        // map 1 at the same even offset.
        let v = VgaText::new();
        assert_eq!(v.planes.len(), VGA_PLANE_COUNT * VGA_PLANE_SIZE);
        assert_eq!(v.char_at(0, 0), Some(b' '));
        assert_eq!(v.attr_at(0, 0), Some(0x07));
        assert_eq!(v.char_at(24, 79), Some(b' '));
        assert_eq!(v.attr_at(24, 79), Some(0x07));
        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 0), Some(b' '));
        assert_eq!(v.plane_byte(VGA_TEXT_ATTR_PLANE, 0), Some(0x07));
        // Beyond 80×25 the maps remain 0 after reset, and no font is installed.
        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 80 * 25 * 2), Some(0));
        assert_eq!(v.plane_byte(VGA_TEXT_ATTR_PLANE, 80 * 25 * 2), Some(0));
        assert!((0..VGA_PLANE_SIZE).all(|o| v.plane_byte(VGA_FONT_PLANE, o) == Some(0)));
        assert_eq!(v.crtc_index, 0);
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_HIGH)],
            VGA_CRTC_START_ADDR_HIGH_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_LOW)],
            VGA_CRTC_START_ADDR_LOW_DEFAULT
        );
        assert_eq!(v.text_start_address(), 0);
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)],
            VGA_CRTC_MAX_SCAN_LINE_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_OFFSET)],
            VGA_CRTC_OFFSET_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)],
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT
        );
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.misc_output, VGA_MISC_OUTPUT_DEFAULT);
        assert_eq!(v.dac_pel_mask, VGA_DAC_PEL_MASK_DEFAULT);
        assert_eq!(v.dac_ram, vga_dac_default_ram());
        assert_eq!(v.dac_write_index, 0);
        assert_eq!(v.dac_read_index, 0);
        assert_eq!(v.dac_write_channel, 0);
        assert_eq!(v.dac_read_channel, 0);
        assert_eq!(v.dac_state, VGA_DAC_STATE_WRITE);
    }

    #[test]
    fn b8000_char_attr_round_trip() {
        let mut v = VgaText::new();
        assert!(v.write_u8(VGA_TEXT_BASE, b'H'));
        assert!(v.write_u8(VGA_TEXT_BASE + 1, 0x1E));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'H'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE + 1), Some(0x1E));
        assert_eq!(v.char_at(0, 0), Some(b'H'));
        assert_eq!(v.attr_at(0, 0), Some(0x1E));
    }

    #[test]
    fn attr_byte_is_odd_offset() {
        // Spec: OSDev Text UI — attribute is the odd byte of the cell.
        let mut v = VgaText::new();
        v.put_char(0, 0, b'A', 0x2F);
        assert_eq!(v.read_u8(0xB8001), Some(0x2F));
    }

    #[test]
    fn outside_window_not_owned() {
        let v = VgaText::new();
        assert!(!VgaText::owns_addr(0xA0000));
        assert!(!VgaText::owns_addr(0xC0000));
        assert!(VgaText::owns_addr(0xB8000));
        assert!(VgaText::owns_addr(0xBFFFF));
        assert!(!VgaText::owns_addr(0xC0000));
        assert_eq!(v.read_u8(0xA0000), None);
        assert!(!{
            let mut v = VgaText::new();
            v.write_u8(0xC0000, 0x55)
        });
    }

    #[test]
    fn crtc_index_data_round_trip() {
        // Spec: OSDev VGA Hardware / FreeVGA — write index 0x3D4, data 0x3D5.
        let mut v = VgaText::new();
        assert!(v.owns_port(VGA_CRTC_INDEX));
        assert!(v.owns_port(VGA_CRTC_DATA));
        assert!(!v.owns_port(VGA_CRTC_INDEX_MONO));
        v.port_write(VGA_CRTC_INDEX, 1, 0x0E); // cursor location high
        v.port_write(VGA_CRTC_DATA, 1, 0x12);
        v.port_write(VGA_CRTC_INDEX, 1, 0x0F); // cursor location low
        v.port_write(VGA_CRTC_DATA, 1, 0x34);
        assert_eq!(v.crtc_regs[0x0E], 0x12);
        assert_eq!(v.crtc_regs[0x0F], 0x34);
        v.port_write(VGA_CRTC_INDEX, 1, 0x0E);
        assert_eq!(v.port_read(VGA_CRTC_INDEX, 1) as u8, 0x0E);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x12);
        v.port_write(VGA_CRTC_INDEX, 1, 0x0F);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x34);
    }

    #[test]
    fn crtc_word_write_index_and_data() {
        // Spec: OSDev VGA Hardware — 16-bit write to 0x3D4 (lo=index, hi=data).
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 2, 0xAB_0C);
        assert_eq!(v.crtc_index, 0x0C);
        assert_eq!(v.crtc_regs[0x0C], 0xAB);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0xAB);
    }

    #[test]
    fn crtc_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, 0x20);
        v.port_write(VGA_CRTC_DATA, 1, 0x55);
        // Index 0x20 is beyond the 25 standard registers; data write ignored.
        // In-range mode-03h Max Scan Line / Offset / Underline defaults are unchanged.
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)],
            VGA_CRTC_MAX_SCAN_LINE_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_OFFSET)],
            VGA_CRTC_OFFSET_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)],
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT
        );
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_MAX_SCAN_LINE));
        assert_eq!(
            v.port_read(VGA_CRTC_DATA, 1) as u8,
            VGA_CRTC_MAX_SCAN_LINE_DEFAULT
        );
    }

    #[test]
    fn reset_clears_crtc_regs() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, 0x07);
        v.port_write(VGA_CRTC_DATA, 1, 0x99);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        v.port_write(VGA_CRTC_DATA, 1, 0x12);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        v.port_write(VGA_CRTC_DATA, 1, 0x34);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_MAX_SCAN_LINE));
        v.port_write(VGA_CRTC_DATA, 1, 0x55);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OFFSET));
        v.port_write(VGA_CRTC_DATA, 1, 0x55);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_UNDERLINE_LOCATION));
        v.port_write(VGA_CRTC_DATA, 1, 0x55);
        v.reset();
        assert_eq!(v.crtc_index, 0);
        assert_eq!(v.crtc_regs[0x07], 0);
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_HIGH)],
            VGA_CRTC_START_ADDR_HIGH_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_LOW)],
            VGA_CRTC_START_ADDR_LOW_DEFAULT
        );
        assert_eq!(v.text_start_address(), 0);
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)],
            VGA_CRTC_MAX_SCAN_LINE_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_OFFSET)],
            VGA_CRTC_OFFSET_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)],
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT
        );
    }

    /// Spec: FreeVGA CRT Controller / IBM VGA — Start Address High `0x0C` /
    /// Low `0x0D` form a 16-bit character address of the first displayed cell;
    /// Protect does not cover indexes `>= 0x08`. Mode-03h reset default is
    /// `0x0000`.
    #[test]
    fn crtc_start_address_store_readback_and_helper() {
        let mut v = VgaText::new();
        assert_eq!(v.text_start_address(), 0);
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_HIGH)],
            VGA_CRTC_START_ADDR_HIGH_DEFAULT
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_LOW)],
            VGA_CRTC_START_ADDR_LOW_DEFAULT
        );
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        assert_eq!(
            v.port_read(VGA_CRTC_DATA, 1) as u8,
            VGA_CRTC_START_ADDR_HIGH_DEFAULT
        );
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        assert_eq!(
            v.port_read(VGA_CRTC_DATA, 1) as u8,
            VGA_CRTC_START_ADDR_LOW_DEFAULT
        );

        // Protect set — indexes 0x0C/0x0D must still accept writes.
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        v.port_write(VGA_CRTC_DATA, 1, 0x01);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        v.port_write(VGA_CRTC_DATA, 1, 0x4F); // 0x014F = 335

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x01);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x4F);

        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_HIGH)], 0x01);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_START_ADDR_LOW)], 0x4F);
        assert_eq!(v.text_start_address(), 0x014F);

        // Word write path (lo=index, hi=data) also updates Start Address under Protect.
        v.port_write(VGA_CRTC_INDEX, 2, 0x50_0D); // index 0x0D, data 0x50
        v.port_write(VGA_CRTC_INDEX, 2, 0x00_0C); // index 0x0C, data 0x00
        assert_eq!(v.text_start_address(), 80);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x50);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x00);
    }

    /// Spec: FreeVGA CRT Controller — Start Address is the character index of the
    /// first displayed cell. Host text helpers (`char_at` / `attr_at` / `put_char`)
    /// index the visible 80×25 viewport relative to that origin; CPU MMIO at
    /// `0xB8000` remains an absolute plane aperture (not remapped).
    #[test]
    fn crtc_start_address_offsets_host_text_viewport() {
        let mut v = VgaText::new();
        assert_eq!(v.text_start_plane_offset(), 0);

        // Scroll origin to character 80 (one 80-col row).
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        v.port_write(VGA_CRTC_DATA, 1, 0x00);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        v.port_write(VGA_CRTC_DATA, 1, 80);
        assert_eq!(v.text_start_address(), 80);
        assert_eq!(v.text_start_plane_offset(), 80 * VGA_CELL_BYTES);

        // Absolute CPU write at plane cell 80 appears at visible (0,0).
        let abs = VGA_TEXT_BASE + (80 * VGA_CELL_BYTES) as u64;
        assert!(v.write_u8(abs, b'S'));
        assert!(v.write_u8(abs + 1, 0x1E));
        assert_eq!(v.char_at(0, 0), Some(b'S'));
        assert_eq!(v.attr_at(0, 0), Some(0x1E));

        // Host put_char writes into the scrolled plane origin; MMIO absolute
        // base cell (start=0) stays unchanged.
        assert!(v.put_char(0, 0, b'T', 0x2F));
        assert_eq!(v.read_u8(abs), Some(b'T'));
        assert_eq!(v.read_u8(abs + 1), Some(0x2F));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(VGA_DEFAULT_CHAR));
        assert_eq!(v.read_u8(VGA_TEXT_BASE + 1), Some(VGA_DEFAULT_ATTR));

        // Visible (1,0) is character index 160 under start=80.
        assert!(v.put_char(1, 0, b'U', 0x4E));
        let row1 = VGA_TEXT_BASE + (160 * VGA_CELL_BYTES) as u64;
        assert_eq!(v.read_u8(row1), Some(b'U'));
        assert_eq!(v.char_at(1, 0), Some(b'U'));
    }

    /// Spec: FreeVGA — Start Address writes remain accepted under Protect; reset
    /// restores mode-03h `0x0000` so the host viewport origin returns to plane 0.
    #[test]
    fn crtc_start_address_viewport_respects_protect_and_reset() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        v.port_write(VGA_CRTC_DATA, 1, 0x00);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        v.port_write(VGA_CRTC_DATA, 1, 40);
        assert_eq!(v.text_start_address(), 40);
        assert!(v.put_char(0, 0, b'P', 0x07));
        let abs = VGA_TEXT_BASE + (40 * VGA_CELL_BYTES) as u64;
        assert_eq!(v.read_u8(abs), Some(b'P'));

        v.reset();
        assert_eq!(v.text_start_address(), 0);
        assert_eq!(v.text_start_plane_offset(), 0);
        assert_eq!(v.char_at(0, 0), Some(VGA_DEFAULT_CHAR));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(VGA_DEFAULT_CHAR));
    }

    /// Spec: FreeVGA CRT Controller — Maximum Scan Line (index `0x09`)
    /// store/readback; Protect does not cover indexes `>= 0x08`. Mode-03h reset
    /// default is [`VGA_CRTC_MAX_SCAN_LINE_DEFAULT`] (`0x0F`).
    #[test]
    fn crtc_max_scan_line_store_readback_with_protect() {
        let mut v = VgaText::new();
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)],
            VGA_CRTC_MAX_SCAN_LINE_DEFAULT
        );
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_MAX_SCAN_LINE));
        assert_eq!(
            v.port_read(VGA_CRTC_DATA, 1) as u8,
            VGA_CRTC_MAX_SCAN_LINE_DEFAULT
        );

        // Protect set — index 0x09 must still accept writes.
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        let programmed = VGA_CRTC_MAX_SCAN_MASK // MaxScan = 0x1F
            | VGA_CRTC_MAX_SCAN_START_VBLANK_BIT9
            | VGA_CRTC_MAX_SCAN_LINE_COMPARE_BIT9
            | VGA_CRTC_MAX_SCAN_DOUBLING;
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_MAX_SCAN_LINE));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(programmed));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, programmed);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)], programmed);

        // Word write path (lo=index, hi=data) also updates Max Scan under Protect.
        v.port_write(VGA_CRTC_INDEX, 2, 0x4E_09); // index 0x09, data 0x4E
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)], 0x4E);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_MAX_SCAN_LINE));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x4E);
    }

    /// Spec: FreeVGA CRT Controller — Offset (index `0x13`) store/readback;
    /// Protect does not cover indexes `>= 0x08`. Mode-03h reset default is
    /// [`VGA_CRTC_OFFSET_DEFAULT`] (`0x28` for 80-column text).
    #[test]
    fn crtc_offset_store_readback_with_protect() {
        let mut v = VgaText::new();
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_OFFSET)],
            VGA_CRTC_OFFSET_DEFAULT
        );
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OFFSET));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, VGA_CRTC_OFFSET_DEFAULT);

        // Protect set — index 0x13 must still accept writes.
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        let programmed = 0x50; // 80 words (e.g. wider logical pitch)
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OFFSET));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(programmed));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, programmed);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_OFFSET)], programmed);

        // Word write path (lo=index, hi=data) also updates Offset under Protect.
        v.port_write(VGA_CRTC_INDEX, 2, 0x40_13); // index 0x13, data 0x40
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_OFFSET)], 0x40);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OFFSET));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x40);
    }

    /// Spec: FreeVGA CRT Controller — Offset is logical line width in words;
    /// host text helpers convert words→character cells (`Offset * 2`). Mode-03h
    /// reset `0x28` → 80-character row stride (identity with [`VGA_TEXT_COLS`]).
    #[test]
    fn crtc_offset_default_pitch_matches_80_col_stride() {
        let mut v = VgaText::new();
        assert_eq!(v.text_row_pitch_chars(), VGA_TEXT_COLS);
        assert_eq!(
            v.text_row_pitch_chars(),
            usize::from(VGA_CRTC_OFFSET_DEFAULT) * 2
        );

        assert!(v.put_char(1, 0, b'R', 0x1E));
        let row1 = VGA_TEXT_BASE + (VGA_TEXT_COLS * VGA_CELL_BYTES) as u64;
        assert_eq!(v.read_u8(row1), Some(b'R'));
        assert_eq!(v.read_u8(row1 + 1), Some(0x1E));
        assert_eq!(v.char_at(1, 0), Some(b'R'));
        assert_eq!(v.attr_at(1, 0), Some(0x1E));
        // Absolute base cell unchanged.
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(VGA_DEFAULT_CHAR));
    }

    /// Spec: FreeVGA — non-default Offset widens logical pitch so adjacent
    /// character rows are farther apart in the refresh buffer. Host
    /// `char_at`/`attr_at`/`put_char` use that stride; CPU `0xB8000` MMIO stays
    /// absolute.
    #[test]
    fn crtc_offset_nondefault_pitch_changes_host_text_row_stride() {
        let mut v = VgaText::new();
        // Offset 0x50 = 80 words → 160 character cells between rows.
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OFFSET));
        v.port_write(VGA_CRTC_DATA, 1, 0x50);
        assert_eq!(v.text_row_pitch_chars(), 160);

        assert!(v.put_char(0, 0, b'A', 0x07));
        assert!(v.put_char(1, 0, b'B', 0x1F));
        assert!(v.put_char(2, 3, b'C', 0x2E));

        let row0 = VGA_TEXT_BASE;
        let row1 = VGA_TEXT_BASE + (160 * VGA_CELL_BYTES) as u64;
        let row2_col3 = VGA_TEXT_BASE + ((160 * 2 + 3) * VGA_CELL_BYTES) as u64;
        assert_eq!(v.read_u8(row0), Some(b'A'));
        assert_eq!(v.read_u8(row1), Some(b'B'));
        assert_eq!(v.read_u8(row1 + 1), Some(0x1F));
        assert_eq!(v.read_u8(row2_col3), Some(b'C'));
        assert_eq!(v.read_u8(row2_col3 + 1), Some(0x2E));

        // Classic 80-col neighbor is not row 1 under wide pitch.
        let classic_row1 = VGA_TEXT_BASE + (VGA_TEXT_COLS * VGA_CELL_BYTES) as u64;
        assert_eq!(v.read_u8(classic_row1), Some(VGA_DEFAULT_CHAR));

        assert_eq!(v.char_at(1, 0), Some(b'B'));
        assert_eq!(v.attr_at(1, 0), Some(0x1F));
        assert_eq!(v.char_at(2, 3), Some(b'C'));
        assert_eq!(v.attr_at(2, 3), Some(0x2E));
    }

    /// Spec: FreeVGA — Offset pitch combines with Start Address: visible cell
    /// `(row, col)` is at character index `start + row*pitch + col`.
    #[test]
    fn crtc_offset_pitch_combines_with_start_address() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_HIGH));
        v.port_write(VGA_CRTC_DATA, 1, 0x00);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_START_ADDR_LOW));
        v.port_write(VGA_CRTC_DATA, 1, 10);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OFFSET));
        v.port_write(VGA_CRTC_DATA, 1, 0x40); // 64 words → 128 chars/row
        assert_eq!(v.text_start_address(), 10);
        assert_eq!(v.text_row_pitch_chars(), 128);

        assert!(v.put_char(1, 2, b'X', 0x4E));
        let abs = VGA_TEXT_BASE + ((10 + 128 + 2) * VGA_CELL_BYTES) as u64;
        assert_eq!(v.read_u8(abs), Some(b'X'));
        assert_eq!(v.read_u8(abs + 1), Some(0x4E));
        assert_eq!(v.char_at(1, 2), Some(b'X'));
        assert_eq!(v.attr_at(1, 2), Some(0x4E));
    }

    /// Spec: FreeVGA — Offset remains writable under Protect; reset restores
    /// mode-03h `0x28` pitch (80-col stride).
    #[test]
    fn crtc_offset_pitch_respects_protect_and_reset() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OFFSET));
        v.port_write(VGA_CRTC_DATA, 1, 0x50);
        assert_eq!(v.text_row_pitch_chars(), 160);
        assert!(v.put_char(1, 0, b'P', 0x07));
        let wide = VGA_TEXT_BASE + (160 * VGA_CELL_BYTES) as u64;
        assert_eq!(v.read_u8(wide), Some(b'P'));

        v.reset();
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_OFFSET)],
            VGA_CRTC_OFFSET_DEFAULT
        );
        assert_eq!(v.text_row_pitch_chars(), VGA_TEXT_COLS);
        assert_eq!(v.char_at(1, 0), Some(VGA_DEFAULT_CHAR));
        assert_eq!(
            v.read_u8(VGA_TEXT_BASE + (VGA_TEXT_COLS * VGA_CELL_BYTES) as u64),
            Some(VGA_DEFAULT_CHAR)
        );
    }

    /// Spec: FreeVGA CRT Controller — Underline Location (index `0x14`)
    /// store/readback; Protect does not cover indexes `>= 0x08`. Mode-03h reset
    /// default is [`VGA_CRTC_UNDERLINE_LOCATION_DEFAULT`] (`0x1F`).
    #[test]
    fn crtc_underline_location_store_readback_with_protect() {
        let mut v = VgaText::new();
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)],
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT
        );
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_UNDERLINE_LOCATION));
        assert_eq!(
            v.port_read(VGA_CRTC_DATA, 1) as u8,
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT
        );

        // Protect set — index 0x14 must still accept writes.
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        let programmed = 0x0D // Underline Location field
            | VGA_CRTC_UNDERLINE_DIV4
            | VGA_CRTC_UNDERLINE_DW;
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_UNDERLINE_LOCATION));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(programmed));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, programmed);
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)],
            programmed
        );

        // Word write path (lo=index, hi=data) also updates Underline under Protect.
        v.port_write(VGA_CRTC_INDEX, 2, 0x55_14); // index 0x14, data 0x55
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)], 0x55);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_UNDERLINE_LOCATION));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x55);
    }

    #[test]
    fn crtc_cursor_location_store_readback_and_offset() {
        // Spec: FreeVGA CRT Controller / IBM VGA — Cursor Location High `0x0E`
        // and Low `0x0F` form a 16-bit character address; text cells are 2 bytes.
        let mut v = VgaText::new();
        assert_eq!(v.crtc_cursor_location(), 0);
        assert_eq!(v.crtc_cursor_plane_offset(), 0);
        assert_eq!(v.crtc_cursor_row_col(), (0, 0));

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_LOC_HIGH));
        v.port_write(VGA_CRTC_DATA, 1, 0x01);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_LOC_LOW));
        v.port_write(VGA_CRTC_DATA, 1, 0x4F); // 0x014F = 335 → row 4, col 15

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_LOC_HIGH));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x01);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_LOC_LOW));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x4F);

        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_CURSOR_LOC_HIGH)], 0x01);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_CURSOR_LOC_LOW)], 0x4F);
        assert_eq!(v.crtc_cursor_location(), 0x014F);
        assert_eq!(v.crtc_cursor_plane_offset(), 0x014F * VGA_CELL_BYTES);
        assert_eq!(v.crtc_cursor_row_col(), (4, 15));
    }

    #[test]
    fn crtc_cursor_location_word_write_and_row_col() {
        // Spec: FreeVGA — 16-bit write to 0x3D4 (lo=index, hi=data); location
        // is a character index (row = loc/80, col = loc%80 for mode 03h).
        let mut v = VgaText::new();
        // Character 80 → start of row 1.
        v.port_write(VGA_CRTC_INDEX, 2, 0x50_0F); // index 0x0F, data 0x50
        v.port_write(VGA_CRTC_INDEX, 2, 0x00_0E); // index 0x0E, data 0x00
        assert_eq!(v.crtc_cursor_location(), 80);
        assert_eq!(v.crtc_cursor_row_col(), (1, 0));
        assert_eq!(v.crtc_cursor_plane_offset(), 160);
    }

    #[test]
    fn crtc_cursor_start_end_and_disable() {
        // Spec: FreeVGA CRT Controller — Cursor Start `0x0A` bits 4:0 start
        // scanline, bit5 Cursor Disable; Cursor End `0x0B` bits 4:0 end.
        let mut v = VgaText::new();
        assert!(!v.crtc_cursor_disabled());
        assert_eq!(v.crtc_cursor_start_scanline(), 0);
        assert_eq!(v.crtc_cursor_end_scanline(), 0);

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_START));
        v.port_write(VGA_CRTC_DATA, 1, 0x0E); // underline-ish start, CD clear
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_END));
        v.port_write(VGA_CRTC_DATA, 1, 0x0F);

        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x0F);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_START));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x0E);
        assert_eq!(v.crtc_cursor_start_scanline(), 0x0E);
        assert_eq!(v.crtc_cursor_end_scanline(), 0x0F);
        assert!(!v.crtc_cursor_disabled());

        // Bit5 CD disables cursor; scanline field still readable via mask.
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_CURSOR_DISABLE | 0x0A));
        assert!(v.crtc_cursor_disabled());
        assert_eq!(v.crtc_cursor_start_scanline(), 0x0A);
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_CURSOR_START)],
            VGA_CRTC_CURSOR_DISABLE | 0x0A
        );
    }

    #[test]
    fn crtc_cursor_regs_cleared_on_reset() {
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 2, 0x12_0E);
        v.port_write(VGA_CRTC_INDEX, 2, 0x34_0F);
        v.port_write(VGA_CRTC_INDEX, 2, 0x2E_0A);
        v.port_write(VGA_CRTC_INDEX, 2, 0x0F_0B);
        assert_eq!(v.crtc_cursor_location(), 0x1234);
        assert!(v.crtc_cursor_disabled());

        v.reset();
        assert_eq!(v.crtc_cursor_location(), 0);
        assert_eq!(v.crtc_cursor_plane_offset(), 0);
        assert!(!v.crtc_cursor_disabled());
        assert_eq!(v.crtc_cursor_start_scanline(), 0);
        assert_eq!(v.crtc_cursor_end_scanline(), 0);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_CURSOR_START)], 0);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_CURSOR_END)], 0);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_CURSOR_LOC_HIGH)], 0);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_CURSOR_LOC_LOW)], 0);
    }

    #[test]
    fn crtc_protect_blocks_writes_to_indexes_0_through_7() {
        // Spec: FreeVGA CRT Controller / IBM VGA — Vertical Retrace End `0x11`
        // bit7 Protect: when set, CRTC indexes `0x00`–`0x07` ignore writes
        // (readback unchanged). Firmware clears Protect before programming
        // horizontal/vertical timing regs.
        let mut v = VgaText::new();
        for idx in 0u8..=0x07 {
            v.port_write(VGA_CRTC_INDEX, 1, u32::from(idx));
            v.port_write(VGA_CRTC_DATA, 1, 0x5A);
            assert_eq!(v.crtc_regs[usize::from(idx)], 0x5A);
        }

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_VERTICAL_RETRACE_END)] & VGA_CRTC_PROTECT,
            VGA_CRTC_PROTECT
        );

        for idx in 0u8..=0x07 {
            v.port_write(VGA_CRTC_INDEX, 1, u32::from(idx));
            v.port_write(VGA_CRTC_DATA, 1, 0xA5);
            // Overflow bit4 exception covered separately; other bits stay 0x5A.
            if idx == VGA_CRTC_OVERFLOW {
                assert_eq!(
                    v.crtc_regs[usize::from(idx)] & !VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8,
                    0x5A & !VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8
                );
            } else {
                assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x5A);
                assert_eq!(v.crtc_regs[usize::from(idx)], 0x5A);
            }
        }
    }

    #[test]
    fn crtc_protect_cleared_allows_timing_reg_writes() {
        // Spec: FreeVGA — clearing Protect (index `0x11` bit7) restores writes
        // to indexes `0x00`–`0x07`.
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));
        v.port_write(VGA_CRTC_INDEX, 1, 0x00);
        v.port_write(VGA_CRTC_DATA, 1, 0x11);
        assert_eq!(v.crtc_regs[0x00], 0);

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, 0x00); // clear Protect
        v.port_write(VGA_CRTC_INDEX, 1, 0x00);
        v.port_write(VGA_CRTC_DATA, 1, 0x11);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x11);
        assert_eq!(v.crtc_regs[0x00], 0x11);

        v.port_write(VGA_CRTC_INDEX, 1, 0x06);
        v.port_write(VGA_CRTC_DATA, 1, 0x22);
        assert_eq!(v.crtc_regs[0x06], 0x22);
    }

    #[test]
    fn crtc_protect_does_not_block_indexes_above_7() {
        // Spec: FreeVGA Protect only covers indexes `0x00`–`0x07`; cursor and
        // other CRTC regs remain writable (index `0x11` itself always writable).
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT | 0x0E));

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_LOC_HIGH));
        v.port_write(VGA_CRTC_DATA, 1, 0x01);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_CURSOR_LOC_LOW));
        v.port_write(VGA_CRTC_DATA, 1, 0x4F);
        assert_eq!(v.crtc_cursor_location(), 0x014F);

        v.port_write(VGA_CRTC_INDEX, 1, 0x12); // Vertical Display End
        v.port_write(VGA_CRTC_DATA, 1, 0x8F);
        assert_eq!(v.crtc_regs[0x12], 0x8F);

        // Word write path (lo=index, hi=data) also honors Protect scope.
        v.port_write(VGA_CRTC_INDEX, 2, 0xAB_08);
        assert_eq!(v.crtc_regs[0x08], 0xAB);
        v.port_write(VGA_CRTC_INDEX, 2, 0xCD_00); // protected index — ignored
        assert_eq!(v.crtc_regs[0x00], 0);
    }

    #[test]
    fn crtc_protect_overflow_line_compare_bit_still_writable() {
        // Spec: FreeVGA Vertical Retrace End Protect — indexes `0x00`–`0x07`
        // ignore writes except Overflow (`0x07`) bit4 (Line Compare bit8).
        let mut v = VgaText::new();
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OVERFLOW));
        v.port_write(VGA_CRTC_DATA, 1, 0x0F); // bits 3:0 set, bit4 clear
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));

        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OVERFLOW));
        v.port_write(VGA_CRTC_DATA, 1, 0xF0); // attempt to set bit4 + clear low nybble
        assert_eq!(
            v.port_read(VGA_CRTC_DATA, 1) as u8,
            0x0F | VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_OVERFLOW)],
            0x0F | VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8
        );

        // Clearing bit4 under Protect also works; other bits stay.
        v.port_write(VGA_CRTC_DATA, 1, 0x00);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_OVERFLOW)], 0x0F);
    }

    /// Spec: FreeVGA CRT Controller — Overflow (index [`VGA_CRTC_OVERFLOW`] =
    /// `0x07`) store/readback of FreeVGA high-bit fields; under Protect only
    /// bit4 ([`VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8`] / Line Compare bit8)
    /// remains writable.
    #[test]
    fn crtc_overflow_store_readback_line_compare_under_protect() {
        assert_eq!(VGA_CRTC_OVERFLOW, 0x07);
        let mut v = VgaText::new();
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_OVERFLOW)], 0);

        // Unlocked: full FreeVGA Overflow bitfield store/readback.
        let all_bits = VGA_CRTC_OVERFLOW_VT_BIT8
            | VGA_CRTC_OVERFLOW_VDE_BIT8
            | VGA_CRTC_OVERFLOW_VRS_BIT8
            | VGA_CRTC_OVERFLOW_START_VBLANK_BIT8
            | VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8
            | VGA_CRTC_OVERFLOW_VT_BIT9
            | VGA_CRTC_OVERFLOW_VDE_BIT9
            | VGA_CRTC_OVERFLOW_VRS_BIT9;
        assert_eq!(all_bits, 0xFF);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OVERFLOW));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(all_bits));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, all_bits);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_OVERFLOW)], all_bits);

        // Seed non-LC bits, clear Line Compare bit8, then enable Protect.
        let seed = VGA_CRTC_OVERFLOW_VT_BIT8
            | VGA_CRTC_OVERFLOW_VDE_BIT8
            | VGA_CRTC_OVERFLOW_VRS_BIT8
            | VGA_CRTC_OVERFLOW_START_VBLANK_BIT8
            | VGA_CRTC_OVERFLOW_VT_BIT9;
        v.port_write(VGA_CRTC_DATA, 1, u32::from(seed));
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, seed);
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_VERTICAL_RETRACE_END));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(VGA_CRTC_PROTECT));

        // Under Protect: only Line Compare bit8 (Overflow bit4) updates; other
        // FreeVGA Overflow bits stay at the pre-Protect seed.
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OVERFLOW));
        v.port_write(
            VGA_CRTC_DATA,
            1,
            u32::from(
                VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8
                    | VGA_CRTC_OVERFLOW_VDE_BIT9
                    | VGA_CRTC_OVERFLOW_VRS_BIT9,
            ),
        );
        let expected = seed | VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8;
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, expected);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_OVERFLOW)], expected);

        // Clear Line Compare bit8 under Protect; non-LC seed bits preserved.
        v.port_write(VGA_CRTC_DATA, 1, 0x00);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, seed);
        assert_eq!(v.crtc_regs[usize::from(VGA_CRTC_OVERFLOW)], seed);

        // Word write path (lo=index, hi=data) also updates only bit4 under Protect.
        v.port_write(
            VGA_CRTC_INDEX,
            2,
            u32::from(VGA_CRTC_OVERFLOW)
                | (u32::from(VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8 | VGA_CRTC_OVERFLOW_VRS_BIT9)
                    << 8),
        );
        assert_eq!(
            v.crtc_regs[usize::from(VGA_CRTC_OVERFLOW)],
            seed | VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8
        );
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(VGA_CRTC_OVERFLOW));
        assert_eq!(
            v.port_read(VGA_CRTC_DATA, 1) as u8,
            seed | VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8
        );
    }

    #[test]
    fn misc_output_owns_ports_color_crtc_by_default() {
        // Spec: FreeVGA / OSDev — Misc Output write `0x3C2`, read `0x3CC`;
        // default IOAS=1 owns color CRTC `0x3D4`/`0x3D5`; mono `0x3B4`/`0x3B5`
        // not owned until IOAS is cleared.
        let v = VgaText::new();
        assert!(v.owns_port(VGA_MISC_OUTPUT_WRITE));
        assert!(v.owns_port(VGA_MISC_OUTPUT_READ));
        assert!(v.owns_port(VGA_CRTC_INDEX));
        assert!(v.owns_port(VGA_CRTC_DATA));
        assert!(!v.owns_port(VGA_CRTC_INDEX_MONO));
        assert!(!v.owns_port(VGA_CRTC_DATA_MONO));
    }

    #[test]
    fn crtc_mono_ports_owned_when_misc_ioas_cleared() {
        // Spec: FreeVGA / IBM VGA Misc Output bit0 (IOAS) — clear selects mono
        // I/O map; CRTC Address/Data move to `0x3B4`/`0x3B5`. Color `0x3D4`/
        // `0x3D5` are not owned.
        let mut v = VgaText::new();
        assert!(v.misc_ioas_color());
        assert!(v.owns_port(VGA_CRTC_INDEX));
        assert!(v.owns_port(VGA_CRTC_DATA));
        assert!(!v.owns_port(VGA_CRTC_INDEX_MONO));
        assert!(!v.owns_port(VGA_CRTC_DATA_MONO));

        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        assert!(!v.misc_ioas_color());
        assert!(!v.owns_port(VGA_CRTC_INDEX));
        assert!(!v.owns_port(VGA_CRTC_DATA));
        assert!(v.owns_port(VGA_CRTC_INDEX_MONO));
        assert!(v.owns_port(VGA_CRTC_DATA_MONO));
    }

    #[test]
    fn crtc_mono_index_data_round_trip_same_register_file() {
        // Spec: FreeVGA CRT Controller / Misc Output IOAS — mono `0x3B4`/`0x3B5`
        // address the same CRTC register file as color `0x3D4`/`0x3D5`.
        let mut v = VgaText::new();
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );

        v.port_write(VGA_CRTC_INDEX_MONO, 1, 0x0E);
        v.port_write(VGA_CRTC_DATA_MONO, 1, 0x12);
        v.port_write(VGA_CRTC_INDEX_MONO, 1, 0x0F);
        v.port_write(VGA_CRTC_DATA_MONO, 1, 0x34);
        assert_eq!(v.crtc_regs[0x0E], 0x12);
        assert_eq!(v.crtc_regs[0x0F], 0x34);

        v.port_write(VGA_CRTC_INDEX_MONO, 1, 0x0E);
        assert_eq!(v.port_read(VGA_CRTC_INDEX_MONO, 1) as u8, 0x0E);
        assert_eq!(v.port_read(VGA_CRTC_DATA_MONO, 1) as u8, 0x12);
        v.port_write(VGA_CRTC_INDEX_MONO, 1, 0x0F);
        assert_eq!(v.port_read(VGA_CRTC_DATA_MONO, 1) as u8, 0x34);

        // Inactive color alias must not touch the shared file.
        assert_eq!(v.port_read(VGA_CRTC_INDEX, 1), 0xFFFF_FFFF);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1), 0xFFFF_FFFF);
        v.port_write(VGA_CRTC_INDEX, 1, 0x0E);
        v.port_write(VGA_CRTC_DATA, 1, 0x99);
        assert_eq!(v.crtc_regs[0x0E], 0x12);

        // Switching back to color sees the same register file.
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, u32::from(VGA_MISC_OUTPUT_DEFAULT));
        v.port_write(VGA_CRTC_INDEX, 1, 0x0E);
        assert_eq!(v.port_read(VGA_CRTC_DATA, 1) as u8, 0x12);
        assert!(!v.owns_port(VGA_CRTC_INDEX_MONO));
        assert_eq!(v.port_read(VGA_CRTC_INDEX_MONO, 1), 0xFFFF_FFFF);
    }

    #[test]
    fn crtc_mono_word_write_index_and_data() {
        // Spec: OSDev VGA Hardware — 16-bit write to CRTC index (lo=index,
        // hi=data) also applies on the mono map at `0x3B4`.
        let mut v = VgaText::new();
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        v.port_write(VGA_CRTC_INDEX_MONO, 2, 0xAB_0C);
        assert_eq!(v.crtc_index, 0x0C);
        assert_eq!(v.crtc_regs[0x0C], 0xAB);
        assert_eq!(v.port_read(VGA_CRTC_DATA_MONO, 1) as u8, 0xAB);
    }

    #[test]
    fn crtc_cursor_helpers_via_mono_ports() {
        // Spec: FreeVGA cursor location + Misc IOAS — helpers read the shared
        // CRTC file whether programmed through color or mono ports.
        let mut v = VgaText::new();
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );

        v.port_write(VGA_CRTC_INDEX_MONO, 1, u32::from(VGA_CRTC_CURSOR_LOC_HIGH));
        v.port_write(VGA_CRTC_DATA_MONO, 1, 0x01);
        v.port_write(VGA_CRTC_INDEX_MONO, 1, u32::from(VGA_CRTC_CURSOR_LOC_LOW));
        v.port_write(VGA_CRTC_DATA_MONO, 1, 0x4F); // 0x014F → row 4, col 15
        v.port_write(VGA_CRTC_INDEX_MONO, 2, 0x0E_0A); // start scanline 0x0E
        v.port_write(VGA_CRTC_INDEX_MONO, 2, 0x0F_0B); // end scanline 0x0F

        assert_eq!(v.crtc_cursor_location(), 0x014F);
        assert_eq!(v.crtc_cursor_plane_offset(), 0x014F * VGA_CELL_BYTES);
        assert_eq!(v.crtc_cursor_row_col(), (4, 15));
        assert_eq!(v.crtc_cursor_start_scanline(), 0x0E);
        assert_eq!(v.crtc_cursor_end_scanline(), 0x0F);
        assert!(!v.crtc_cursor_disabled());
    }

    #[test]
    fn misc_output_write_3c2_readback_3cc() {
        // Spec: FreeVGA / OSDev VGA Hardware — write MOR at 0x3C2, read at 0x3CC.
        let mut v = VgaText::new();
        assert_eq!(v.misc_output, VGA_MISC_OUTPUT_DEFAULT);
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8,
            VGA_MISC_OUTPUT_DEFAULT
        );
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, 0xA5);
        assert_eq!(v.misc_output, 0xA5);
        assert_eq!(v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8, 0xA5);
    }

    #[test]
    fn misc_ram_enable_default_allows_plane_rw() {
        // Spec: FreeVGA External Registers / IBM VGA Misc Output — bit1 RAM
        // Enable is set in the mode-03h-class default `0x67`; CPU text-plane
        // R/W at `0xB8000` works as today.
        let mut v = VgaText::new();
        assert_eq!(v.misc_output, VGA_MISC_OUTPUT_DEFAULT);
        assert_eq!(v.misc_output & VGA_MISC_RAM_ENABLE, VGA_MISC_RAM_ENABLE);
        assert!(v.misc_ram_enable());
        assert!(v.write_u8(VGA_TEXT_BASE, b'A'));
        assert!(v.write_u8(VGA_TEXT_BASE + 1, 0x1E));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'A'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE + 1), Some(0x1E));
        assert_eq!(v.char_at(0, 0), Some(b'A'));
        assert_eq!(v.attr_at(0, 0), Some(0x1E));
    }

    #[test]
    fn misc_ram_enable_clear_blocks_plane_rw() {
        // Spec: FreeVGA External Registers / IBM VGA Misc Output bit1 — when
        // RAM Enable is clear, CPU accesses to video RAM are disabled.
        // Choice: `read_u8` → `None`, `write_u8` → `false` (same as out-of-window)
        // so `MachineBus` falls through to open-bus / PhysMem; plane contents
        // are unchanged.
        let mut v = VgaText::new();
        assert!(v.write_u8(VGA_TEXT_BASE, b'Z'));
        assert!(v.write_u8(VGA_TEXT_BASE + 1, 0x2F));
        assert_eq!(v.char_at(0, 0), Some(b'Z'));

        let disabled = VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_RAM_ENABLE;
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, u32::from(disabled));
        assert!(!v.misc_ram_enable());
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8 & VGA_MISC_RAM_ENABLE,
            0
        );

        assert!(!v.write_u8(VGA_TEXT_BASE, b'X'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), None);
        assert_eq!(v.read_u8(VGA_TEXT_BASE + 1), None);
        // Plane unchanged (helpers bypass the CPU gate for test inspection).
        assert_eq!(v.char_at(0, 0), Some(b'Z'));
        assert_eq!(v.attr_at(0, 0), Some(0x2F));
    }

    #[test]
    fn misc_ram_enable_reenable_restores_plane_rw() {
        // Spec: FreeVGA / IBM — clearing then setting bit1 restores CPU plane
        // access; `0x3CC` readback reflects RAM Enable.
        let mut v = VgaText::new();
        assert!(v.write_u8(VGA_TEXT_BASE, b'Q'));

        let disabled = VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_RAM_ENABLE;
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, u32::from(disabled));
        assert!(!v.write_u8(VGA_TEXT_BASE, b'N'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), None);
        assert_eq!(v.char_at(0, 0), Some(b'Q'));

        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, u32::from(VGA_MISC_OUTPUT_DEFAULT));
        assert!(v.misc_ram_enable());
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8 & VGA_MISC_RAM_ENABLE,
            VGA_MISC_RAM_ENABLE
        );
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'Q'));
        assert!(v.write_u8(VGA_TEXT_BASE, b'R'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'R'));
        assert_eq!(v.char_at(0, 0), Some(b'R'));
    }

    #[test]
    fn misc_output_3c2_read_is_open_bus() {
        // Spec: FreeVGA / OSDev — 0x3C2 is write-only; read undefined.
        // Choice: return 0xFF (open-bus style); guests must use 0x3CC.
        let mut v = VgaText::new();
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, 0x67);
        assert_eq!(v.port_read(VGA_MISC_OUTPUT_WRITE, 1) as u8, 0xFF);
        assert_eq!(v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8, 0x67);
    }

    #[test]
    fn reset_restores_misc_output_default() {
        let mut v = VgaText::new();
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, 0x12);
        assert_eq!(v.misc_output, 0x12);
        v.reset();
        assert_eq!(v.misc_output, VGA_MISC_OUTPUT_DEFAULT);
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8,
            VGA_MISC_OUTPUT_DEFAULT
        );
    }

    /// Spec: FreeVGA / IBM VGA — Misc Output bits 6/7 HSYNC/VSYNC polarity
    /// store/readback (timing side effects not modeled).
    #[test]
    fn misc_hsync_vsync_polarity_store_readback() {
        let mut v = VgaText::new();
        // Default `0x67` has HSYNC=1 VSYNC=0.
        assert!(v.misc_hsync_polarity());
        assert!(!v.misc_vsync_polarity());
        let cleared = v.misc_output & !(VGA_MISC_HSYNC_POLARITY | VGA_MISC_VSYNC_POLARITY);
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(cleared | VGA_MISC_VSYNC_POLARITY),
        );
        assert!(!v.misc_hsync_polarity());
        assert!(v.misc_vsync_polarity());
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8
                & (VGA_MISC_HSYNC_POLARITY | VGA_MISC_VSYNC_POLARITY),
            VGA_MISC_VSYNC_POLARITY
        );
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(cleared | VGA_MISC_HSYNC_POLARITY | VGA_MISC_VSYNC_POLARITY),
        );
        assert!(v.misc_hsync_polarity());
        assert!(v.misc_vsync_polarity());
    }

    /// Spec: FreeVGA / IBM VGA — Misc Output bits 3:2 Clock Select store/readback.
    #[test]
    fn misc_clock_select_bits_store_readback() {
        let mut v = VgaText::new();
        // Default `0x67` has clock select = 01b (28 MHz class).
        assert_eq!(v.misc_clock_select(), VGA_MISC_CLOCK_28MHZ);
        let base = v.misc_output & !VGA_MISC_CLOCK_SELECT;
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(base | VGA_MISC_CLOCK_25MHZ),
        );
        assert_eq!(v.misc_clock_select(), VGA_MISC_CLOCK_25MHZ);
        assert_eq!(
            v.port_read(VGA_MISC_OUTPUT_READ, 1) as u8 & VGA_MISC_CLOCK_SELECT,
            VGA_MISC_CLOCK_25MHZ
        );
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(base | VGA_MISC_CLOCK_28MHZ),
        );
        assert_eq!(v.misc_clock_select(), VGA_MISC_CLOCK_28MHZ);
    }

    #[test]
    fn sequencer_owns_ports() {
        // Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — Sequencer Address
        // `0x3C4`, Data `0x3C5`.
        let v = VgaText::new();
        assert!(v.owns_port(VGA_SEQ_INDEX));
        assert!(v.owns_port(VGA_SEQ_DATA));
    }

    #[test]
    fn graphics_controller_owns_ports() {
        // Spec: FreeVGA / OSDev VGA Hardware / IBM VGA — GC Address `0x3CE`,
        // Data `0x3CF`.
        let v = VgaText::new();
        assert!(v.owns_port(VGA_GC_INDEX));
        assert!(v.owns_port(VGA_GC_DATA));
    }

    #[test]
    fn graphics_controller_index_data_round_trip() {
        // Spec: FreeVGA Graphics Registers — write index 0x3CE, data 0x3CF;
        // indexes 0x00–0x08 (Set/Reset … Bit Mask).
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        v.port_write(VGA_GC_DATA, 1, 0x00);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_BIT_MASK));
        v.port_write(VGA_GC_DATA, 1, 0xAA);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MODE)], 0x00);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_BIT_MASK)], 0xAA);
        assert_eq!(v.port_read(VGA_GC_INDEX, 1) as u8, VGA_GC_BIT_MASK);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xAA);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x00);
    }

    /// Spec: FreeVGA Graphics Registers — Enable Set/Reset (index `0x01`)
    /// store/readback. Mode-03h reset default is
    /// [`VGA_GC_ENABLE_SET_RESET_DEFAULT`] (`0x00`).
    #[test]
    fn gc_enable_set_reset_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_ENABLE_SET_RESET)],
            VGA_GC_ENABLE_SET_RESET_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_ENABLE_SET_RESET));
        assert_eq!(
            v.port_read(VGA_GC_DATA, 1) as u8,
            VGA_GC_ENABLE_SET_RESET_DEFAULT
        );

        // Enable Set/Reset on planes 0+1.
        v.port_write(VGA_GC_DATA, 1, 0x03);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x03);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_ENABLE_SET_RESET)], 0x03);

        v.port_write(
            VGA_GC_INDEX,
            2,
            (u32::from(0x0Fu8) << 8) | u32::from(VGA_GC_ENABLE_SET_RESET),
        );
        assert_eq!(v.gc_regs[usize::from(VGA_GC_ENABLE_SET_RESET)], 0x0F);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_ENABLE_SET_RESET));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x0F);

        v.reset();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_ENABLE_SET_RESET)],
            VGA_GC_ENABLE_SET_RESET_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_ENABLE_SET_RESET));
        assert_eq!(
            v.port_read(VGA_GC_DATA, 1) as u8,
            VGA_GC_ENABLE_SET_RESET_DEFAULT
        );
    }

    /// Spec: FreeVGA Graphics Registers — Set/Reset (index `0x00`) store/readback.
    /// Mode-03h reset default is [`VGA_GC_SET_RESET_DEFAULT`] (`0x00`).
    #[test]
    fn gc_set_reset_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_SET_RESET)],
            VGA_GC_SET_RESET_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_SET_RESET));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_SET_RESET_DEFAULT);

        // Planes 0+2 Set/Reset value (bits 3:0).
        v.port_write(VGA_GC_DATA, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x05);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_SET_RESET)], 0x05);

        // Word write path (lo=index, hi=data) also updates Set/Reset.
        v.port_write(
            VGA_GC_INDEX,
            2,
            (u32::from(0x0Fu8) << 8) | u32::from(VGA_GC_SET_RESET),
        );
        assert_eq!(v.gc_regs[usize::from(VGA_GC_SET_RESET)], 0x0F);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_SET_RESET));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x0F);

        v.reset();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_SET_RESET)],
            VGA_GC_SET_RESET_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_SET_RESET));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_SET_RESET_DEFAULT);
    }

    /// Spec: FreeVGA Graphics Registers — Data Rotate / Function Select (index
    /// `0x03`) store/readback. Mode-03h reset default is
    /// [`VGA_GC_DATA_ROTATE_DEFAULT`] (`0x00`).
    #[test]
    fn gc_data_rotate_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_DATA_ROTATE)],
            VGA_GC_DATA_ROTATE_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_DATA_ROTATE));
        assert_eq!(
            v.port_read(VGA_GC_DATA, 1) as u8,
            VGA_GC_DATA_ROTATE_DEFAULT
        );

        // Function Select OR (bits 4:3 = 10) + rotate count 3.
        v.port_write(VGA_GC_DATA, 1, 0x13);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x13);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_DATA_ROTATE)], 0x13);

        // Word write path (lo=index, hi=data) also updates Data Rotate.
        v.port_write(
            VGA_GC_INDEX,
            2,
            (u32::from(0x28u8) << 8) | u32::from(VGA_GC_DATA_ROTATE),
        );
        assert_eq!(v.gc_regs[usize::from(VGA_GC_DATA_ROTATE)], 0x28);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_DATA_ROTATE));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x28);

        v.reset();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_DATA_ROTATE)],
            VGA_GC_DATA_ROTATE_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_DATA_ROTATE));
        assert_eq!(
            v.port_read(VGA_GC_DATA, 1) as u8,
            VGA_GC_DATA_ROTATE_DEFAULT
        );
    }

    /// Spec: FreeVGA Graphics Registers — Bit Mask (index `0x08`) store/readback.
    /// Mode-03h reset default is [`VGA_GC_BIT_MASK_DEFAULT`] (`0xFF`).
    #[test]
    fn gc_bit_mask_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_BIT_MASK)],
            VGA_GC_BIT_MASK_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_BIT_MASK));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_BIT_MASK_DEFAULT);

        v.port_write(VGA_GC_DATA, 1, 0x55);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x55);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_BIT_MASK)], 0x55);

        // Word write path (lo=index, hi=data) also updates Bit Mask.
        v.port_write(
            VGA_GC_INDEX,
            2,
            (u32::from(0xA5u8) << 8) | u32::from(VGA_GC_BIT_MASK),
        );
        assert_eq!(v.gc_regs[usize::from(VGA_GC_BIT_MASK)], 0xA5);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_BIT_MASK));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xA5);

        v.reset();
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_BIT_MASK)],
            VGA_GC_BIT_MASK_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_BIT_MASK));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_BIT_MASK_DEFAULT);
    }

    /// Spec: FreeVGA Graphics Registers — Graphics Mode (index `0x05`)
    /// store/readback. Mode-03h reset default is [`VGA_GC_MODE_DEFAULT`]
    /// (`0x10` = Host Odd/Even).
    #[test]
    fn gc_mode_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MODE)], VGA_GC_MODE_DEFAULT);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_MODE_DEFAULT);

        // Write Mode 2 (bits 1:0 = 10) + Shift256 (bit6) — store/readback only.
        v.port_write(VGA_GC_DATA, 1, 0x42);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x42);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MODE)], 0x42);

        // Word write path (lo=index, hi=data) also updates Graphics Mode.
        v.port_write(
            VGA_GC_INDEX,
            2,
            (u32::from(0x00u8) << 8) | u32::from(VGA_GC_MODE),
        );
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MODE)], 0x00);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x00);

        v.reset();
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MODE)], VGA_GC_MODE_DEFAULT);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_MODE_DEFAULT);
    }

    /// Spec: FreeVGA Graphics Registers — Miscellaneous (index `0x06`)
    /// store/readback. Mode-03h reset default is [`VGA_GC_MISC_DEFAULT`]
    /// (`0x0E` = Chain Odd/Even + Memory Map `B8000`).
    #[test]
    fn gc_misc_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MISC)], VGA_GC_MISC_DEFAULT);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MISC));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_MISC_DEFAULT);

        // Graphics Mode + Chain OE + Memory Map A0000 — store/readback only.
        v.port_write(VGA_GC_DATA, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x05);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MISC)], 0x05);

        // Word write path (lo=index, hi=data) also updates Miscellaneous.
        v.port_write(
            VGA_GC_INDEX,
            2,
            (u32::from(0x00u8) << 8) | u32::from(VGA_GC_MISC),
        );
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MISC)], 0x00);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MISC));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x00);

        v.reset();
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MISC)], VGA_GC_MISC_DEFAULT);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MISC));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_MISC_DEFAULT);
    }

    #[test]
    fn graphics_controller_word_write_index_and_data() {
        // Mirror CRTC/Sequencer: 16-bit write to address port (lo=index, hi=data).
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 2, 0x0F_06);
        assert_eq!(v.gc_index, 0x06);
        assert_eq!(v.gc_regs[0x06], 0x0F);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x0F);
    }

    #[test]
    fn graphics_controller_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 1, 0x09);
        v.port_write(VGA_GC_DATA, 1, 0x55);
        // Index 0x09 is beyond the 9 standard registers; data write ignored,
        // read returns 0 (same CRTC/Sequencer out-of-range policy).
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_MODE_DEFAULT);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_BIT_MASK));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_BIT_MASK_DEFAULT);
    }

    #[test]
    fn graphics_controller_reset_defaults_mode03h() {
        // Spec: FreeVGA / OSDev / IBM VGA mode-03h-class GC programming SeaBIOS
        // probes — Data Rotate `0x00`, Mode `0x10`, Misc `0x0E`, Bit Mask `0xFF`
        // (store/readback only).
        let v = VgaText::new();
        assert_eq!(v.gc_index, 0);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_DATA_ROTATE)],
            VGA_GC_DATA_ROTATE_DEFAULT
        );
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MODE)], VGA_GC_MODE_DEFAULT);
        assert_eq!(v.gc_regs[usize::from(VGA_GC_MISC)], VGA_GC_MISC_DEFAULT);
        assert_eq!(
            v.gc_regs[usize::from(VGA_GC_BIT_MASK)],
            VGA_GC_BIT_MASK_DEFAULT
        );
        assert_eq!(
            v.gc_regs,
            [
                0x00,
                0x00,
                0x00,
                VGA_GC_DATA_ROTATE_DEFAULT,
                0x00,
                VGA_GC_MODE_DEFAULT,
                VGA_GC_MISC_DEFAULT,
                0x00,
                VGA_GC_BIT_MASK_DEFAULT
            ]
        );
    }

    #[test]
    fn reset_restores_graphics_controller_defaults() {
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        v.port_write(VGA_GC_DATA, 1, 0x40);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_DATA_ROTATE));
        v.port_write(VGA_GC_DATA, 1, 0x18);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_BIT_MASK));
        v.port_write(VGA_GC_DATA, 1, 0x00);
        v.reset();
        assert_eq!(v.gc_index, 0);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MODE));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_MODE_DEFAULT);
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_DATA_ROTATE));
        assert_eq!(
            v.port_read(VGA_GC_DATA, 1) as u8,
            VGA_GC_DATA_ROTATE_DEFAULT
        );
        v.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_BIT_MASK));
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, VGA_GC_BIT_MASK_DEFAULT);
    }

    #[test]
    fn sequencer_index_data_round_trip() {
        // Spec: FreeVGA Sequencer Registers — write index 0x3C4, data 0x3C5;
        // indexes 0x00–0x04 (Reset, Clocking Mode, Map Mask, Character Map,
        // Memory Mode).
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        v.port_write(VGA_SEQ_DATA, 1, 0x0F);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MEMORY_MODE));
        v.port_write(VGA_SEQ_DATA, 1, 0x06);
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_MAP_MASK)], 0x0F);
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)], 0x06);
        assert_eq!(v.port_read(VGA_SEQ_INDEX, 1) as u8, VGA_SEQ_MEMORY_MODE);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x06);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x0F);
    }

    /// Spec: FreeVGA Sequencer Registers — Map Mask (index `0x02`) store/readback.
    /// Mode-03h reset default is [`VGA_SEQ_MAP_MASK_DEFAULT`] (`0x03`).
    #[test]
    fn seq_map_mask_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_MAP_MASK)],
            VGA_SEQ_MAP_MASK_DEFAULT
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, VGA_SEQ_MAP_MASK_DEFAULT);

        v.port_write(VGA_SEQ_DATA, 1, 0x0F);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x0F);
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_MAP_MASK)], 0x0F);

        // Word write path (lo=index, hi=data) also updates Map Mask.
        v.port_write(
            VGA_SEQ_INDEX,
            2,
            (u32::from(0x05u8) << 8) | u32::from(VGA_SEQ_MAP_MASK),
        );
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_MAP_MASK)], 0x05);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x05);

        v.reset();
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_MAP_MASK)],
            VGA_SEQ_MAP_MASK_DEFAULT
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, VGA_SEQ_MAP_MASK_DEFAULT);
    }

    /// Spec: FreeVGA Sequencer Registers — Character Map Select (index `0x03`)
    /// store/readback. Mode-03h reset default is
    /// [`VGA_SEQ_CHAR_MAP_SELECT_DEFAULT`] (`0x00`).
    #[test]
    fn seq_char_map_select_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_CHAR_MAP_SELECT)],
            VGA_SEQ_CHAR_MAP_SELECT_DEFAULT
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_CHAR_MAP_SELECT));
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8,
            VGA_SEQ_CHAR_MAP_SELECT_DEFAULT
        );

        // Non-zero Map Select A/B programming (font-map side effects deferred).
        v.port_write(VGA_SEQ_DATA, 1, 0x20);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x20);
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_CHAR_MAP_SELECT)], 0x20);

        // Word write path (lo=index, hi=data) also updates Character Map Select.
        v.port_write(
            VGA_SEQ_INDEX,
            2,
            (u32::from(0x14u8) << 8) | u32::from(VGA_SEQ_CHAR_MAP_SELECT),
        );
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_CHAR_MAP_SELECT)], 0x14);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_CHAR_MAP_SELECT));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x14);

        v.reset();
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_CHAR_MAP_SELECT)],
            VGA_SEQ_CHAR_MAP_SELECT_DEFAULT
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_CHAR_MAP_SELECT));
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8,
            VGA_SEQ_CHAR_MAP_SELECT_DEFAULT
        );
    }

    /// Spec: FreeVGA Sequencer Registers — Memory Mode (index `0x04`)
    /// store/readback. Mode-03h reset default is
    /// [`VGA_SEQ_MEMORY_MODE_DEFAULT`] (`0x02`).
    #[test]
    fn seq_memory_mode_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)],
            VGA_SEQ_MEMORY_MODE_DEFAULT
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MEMORY_MODE));
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8,
            VGA_SEQ_MEMORY_MODE_DEFAULT
        );

        // Extended Memory + Odd/Even (chain-4/odd-even side effects deferred).
        v.port_write(VGA_SEQ_DATA, 1, 0x06);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x06);
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)], 0x06);

        // Word write path (lo=index, hi=data) also updates Memory Mode.
        v.port_write(
            VGA_SEQ_INDEX,
            2,
            (u32::from(0x0Eu8) << 8) | u32::from(VGA_SEQ_MEMORY_MODE),
        );
        assert_eq!(v.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)], 0x0E);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MEMORY_MODE));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x0E);

        v.reset();
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)],
            VGA_SEQ_MEMORY_MODE_DEFAULT
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MEMORY_MODE));
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8,
            VGA_SEQ_MEMORY_MODE_DEFAULT
        );
    }

    /// Spec: FreeVGA — Sequencer Clocking Mode (index `0x01`) bit0 8/9-dot
    /// store/readback (glyph timing not enforced).
    #[test]
    fn sequencer_clocking_mode_8dot_bit_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.seq_regs[VGA_SEQ_CLOCKING_MODE as usize] & VGA_SEQ_CLOCKING_8DOT,
            0,
            "default 9-dot"
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_CLOCKING_MODE));
        v.port_write(VGA_SEQ_DATA, 1, u32::from(VGA_SEQ_CLOCKING_8DOT));
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8 & VGA_SEQ_CLOCKING_8DOT,
            VGA_SEQ_CLOCKING_8DOT
        );
        v.port_write(VGA_SEQ_DATA, 1, 0x00);
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8 & VGA_SEQ_CLOCKING_8DOT,
            0
        );
    }

    #[test]
    fn sequencer_word_write_index_and_data() {
        // Mirror CRTC: 16-bit write to address port (lo=index, hi=data).
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 2, 0x0A_01);
        assert_eq!(v.seq_index, 0x01);
        assert_eq!(v.seq_regs[0x01], 0x0A);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x0A);
    }

    #[test]
    fn sequencer_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, 0x05);
        v.port_write(VGA_SEQ_DATA, 1, 0x55);
        // Index 0x05 is beyond the 5 standard registers; data write ignored,
        // read returns 0 (same CRTC out-of-range policy).
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0);
        // In-range defaults unchanged by the ignored write.
        v.port_write(VGA_SEQ_INDEX, 1, 0x00);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x03);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, VGA_SEQ_MAP_MASK_DEFAULT);
    }

    #[test]
    fn sequencer_reset_defaults_mode03h() {
        // Spec: FreeVGA / IBM VGA mode-03h-class Sequencer programming SeaBIOS
        // probes — Reset `0x03`, Clocking Mode `0x00`, Map Mask
        // [`VGA_SEQ_MAP_MASK_DEFAULT`], Character Map Select
        // [`VGA_SEQ_CHAR_MAP_SELECT_DEFAULT`], Memory Mode
        // [`VGA_SEQ_MEMORY_MODE_DEFAULT`] (store/readback only).
        let v = VgaText::new();
        assert_eq!(v.seq_index, 0);
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_MAP_MASK)],
            VGA_SEQ_MAP_MASK_DEFAULT
        );
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_CHAR_MAP_SELECT)],
            VGA_SEQ_CHAR_MAP_SELECT_DEFAULT
        );
        assert_eq!(
            v.seq_regs[usize::from(VGA_SEQ_MEMORY_MODE)],
            VGA_SEQ_MEMORY_MODE_DEFAULT
        );
        assert_eq!(
            v.seq_regs,
            [
                0x03,
                0x00,
                VGA_SEQ_MAP_MASK_DEFAULT,
                VGA_SEQ_CHAR_MAP_SELECT_DEFAULT,
                VGA_SEQ_MEMORY_MODE_DEFAULT
            ]
        );
    }

    #[test]
    fn reset_restores_sequencer_defaults() {
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        v.port_write(VGA_SEQ_DATA, 1, 0x0F);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_CHAR_MAP_SELECT));
        v.port_write(VGA_SEQ_DATA, 1, 0x20);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MEMORY_MODE));
        v.port_write(VGA_SEQ_DATA, 1, 0x0E);
        v.port_write(VGA_SEQ_INDEX, 1, 0x01);
        v.port_write(VGA_SEQ_DATA, 1, 0x01);
        v.reset();
        assert_eq!(v.seq_index, 0);
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, VGA_SEQ_MAP_MASK_DEFAULT);
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_CHAR_MAP_SELECT));
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8,
            VGA_SEQ_CHAR_MAP_SELECT_DEFAULT
        );
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MEMORY_MODE));
        assert_eq!(
            v.port_read(VGA_SEQ_DATA, 1) as u8,
            VGA_SEQ_MEMORY_MODE_DEFAULT
        );
    }

    #[test]
    fn attribute_controller_owns_ports() {
        // Spec: FreeVGA Attribute Controller — Address/Data `0x3C0`, Data Read
        // `0x3C1`; Input Status #1 `0x3DA` resets the flip-flop (color map).
        let v = VgaText::new();
        assert!(v.misc_ioas_color());
        assert!(v.owns_port(VGA_ATC_ADDRESS_DATA));
        assert!(v.owns_port(VGA_ATC_DATA_READ));
        assert!(v.owns_port(VGA_INPUT_STATUS_1));
        assert!(!v.owns_port(VGA_INPUT_STATUS_1_MONO));
    }

    /// Spec: FreeVGA Attribute Controller Registers / IBM VGA — Overscan Color
    /// (index `0x11`) store/readback via `0x3C0`/`0x3C1`. Mode-03h reset default
    /// is [`VGA_ATC_OVERSCAN_COLOR_DEFAULT`] (`0x00`).
    #[test]
    fn atc_overscan_color_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_OVERSCAN_COLOR)],
            VGA_ATC_OVERSCAN_COLOR_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_OVERSCAN_COLOR));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_OVERSCAN_COLOR_DEFAULT
        );

        // Non-default Overscan Color programming (display side effects deferred).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_OVERSCAN_COLOR));
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x55);
        assert_eq!(v.atc_regs[usize::from(VGA_ATC_OVERSCAN_COLOR)], 0x55);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_OVERSCAN_COLOR));
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x55);

        // Word write path (lo=index, hi=data) also updates Overscan Color.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            2,
            (u32::from(0x00u8) << 8) | u32::from(VGA_ATC_OVERSCAN_COLOR),
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_OVERSCAN_COLOR)],
            VGA_ATC_OVERSCAN_COLOR_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_OVERSCAN_COLOR));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_OVERSCAN_COLOR_DEFAULT
        );

        v.reset();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_OVERSCAN_COLOR)],
            VGA_ATC_OVERSCAN_COLOR_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_OVERSCAN_COLOR));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_OVERSCAN_COLOR_DEFAULT
        );
    }

    /// Spec: FreeVGA Attribute Controller Registers / IBM VGA — Color Plane
    /// Enable (index `0x12`) store/readback via `0x3C0`/`0x3C1`. Mode-03h reset
    /// default is [`VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT`] (`0x0F`).
    #[test]
    fn atc_color_plane_enable_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_PLANE_ENABLE)],
            VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_COLOR_PLANE_ENABLE),
        );
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
        );

        // Non-default Color Plane Enable programming (display side effects deferred).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_COLOR_PLANE_ENABLE),
        );
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x05);
        assert_eq!(v.atc_regs[usize::from(VGA_ATC_COLOR_PLANE_ENABLE)], 0x05);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_COLOR_PLANE_ENABLE),
        );
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x05);

        // Word write path (lo=index, hi=data) also updates Color Plane Enable.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            2,
            (u32::from(0x0Fu8) << 8) | u32::from(VGA_ATC_COLOR_PLANE_ENABLE),
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_PLANE_ENABLE)],
            VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_COLOR_PLANE_ENABLE),
        );
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
        );

        v.reset();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_PLANE_ENABLE)],
            VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_COLOR_PLANE_ENABLE),
        );
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
        );
    }

    /// Spec: FreeVGA Attribute Controller Registers / IBM VGA — Horizontal PEL
    /// Panning (index `0x13`) store/readback via `0x3C0`/`0x3C1`. Mode-03h reset
    /// default is [`VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT`] (`0x08`).
    #[test]
    fn atc_pel_panning_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)],
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );

        // Non-default PEL Panning programming (host text_pel_pan observes decode).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x03);
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)],
            0x03
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x03);

        // Word write path (lo=index, hi=data) also updates PEL Panning.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            2,
            (u32::from(0x08u8) << 8) | u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)],
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );

        v.reset();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)],
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );
    }

    /// Spec: FreeVGA Attribute Controller — Horizontal PEL Panning (index `0x13`)
    /// bits 3:0 Pixel Shift Count for 9-dot alphanumeric text. Mode-03h reset
    /// default `0x08` maps to a zero-pixel left shift on the host text path.
    #[test]
    fn atc_pel_panning_default_is_zero_pixel_shift() {
        let v = VgaText::new();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)],
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );
        assert_eq!(v.text_pel_pan(), 0);
    }

    /// Spec: FreeVGA — in 9-dot text modes the soft-scroll sequence is
    /// register `8` (0 pels), then `0`..=`7` → left shift of `n+1` pels within
    /// the character cell. Host `char_at` stays on the character grid (pan is
    /// sub-cell for render).
    #[test]
    fn atc_pel_panning_host_text_path_decodes_9dot_shift() {
        let mut v = VgaText::new();
        assert!(v.put_char(0, 0, b'A', 0x07));
        assert_eq!(v.char_at(0, 0), Some(b'A'));
        assert_eq!(v.text_pel_pan(), 0);

        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x00);
        assert_eq!(v.text_pel_pan(), 1);
        assert_eq!(v.char_at(0, 0), Some(b'A'));

        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x03);
        assert_eq!(v.text_pel_pan(), 4);
        assert_eq!(v.char_at(0, 0), Some(b'A'));

        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x07);
        assert_eq!(v.text_pel_pan(), 8);
        assert_eq!(v.char_at(0, 0), Some(b'A'));

        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x08);
        assert_eq!(v.text_pel_pan(), 0);
        assert_eq!(v.char_at(0, 0), Some(b'A'));
    }

    /// Spec: FreeVGA — reset restores Horizontal PEL Panning `0x08` (9-dot
    /// zero-shift) so [`VgaText::text_pel_pan`] returns 0.
    #[test]
    fn atc_pel_panning_reset_restores_zero_pixel_shift() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_HORIZONTAL_PEL_PANNING),
        );
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x05);
        assert_eq!(v.text_pel_pan(), 6);
        v.reset();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)],
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );
        assert_eq!(v.text_pel_pan(), 0);
    }

    /// Spec: FreeVGA Attribute Controller Registers / IBM VGA — Color Select
    /// (index `0x14`) store/readback via `0x3C0`/`0x3C1`. Mode-03h reset
    /// default is [`VGA_ATC_COLOR_SELECT_DEFAULT`] (`0x00`).
    #[test]
    fn atc_color_select_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)],
            VGA_ATC_COLOR_SELECT_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_COLOR_SELECT));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_COLOR_SELECT_DEFAULT
        );

        // Non-default Color Select programming (observed by host text helpers).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_COLOR_SELECT));
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x05);
        assert_eq!(v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)], 0x05);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_COLOR_SELECT));
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x05);

        // Word write path (lo=index, hi=data) also updates Color Select.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            2,
            (u32::from(0x00u8) << 8) | u32::from(VGA_ATC_COLOR_SELECT),
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)],
            VGA_ATC_COLOR_SELECT_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_COLOR_SELECT));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_COLOR_SELECT_DEFAULT
        );

        v.reset();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)],
            VGA_ATC_COLOR_SELECT_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_COLOR_SELECT));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_COLOR_SELECT_DEFAULT
        );
    }

    /// Spec: FreeVGA Attribute Controller Registers — Mode Control (index `0x10`)
    /// store/readback via `0x3C0`/`0x3C1`. Mode-03h reset default is
    /// [`VGA_ATC_MODE_CONTROL_DEFAULT`] (`0x0C`).
    #[test]
    fn atc_mode_control_store_readback() {
        let mut v = VgaText::new();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)],
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_MODE_CONTROL_DEFAULT
        );

        // Non-default Mode Control programming (graphics/IPS bits unused here).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x41);
        assert_eq!(v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)], 0x41);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x41);

        // Word write path (lo=index, hi=data) also updates Mode Control.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            2,
            (u32::from(0x0Cu8) << 8) | u32::from(VGA_ATC_MODE_CONTROL),
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)],
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_MODE_CONTROL_DEFAULT
        );

        v.reset();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)],
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
    }

    #[test]
    fn attribute_controller_flip_flop_index_data_round_trip() {
        // Spec: FreeVGA Accessing the Attribute Registers — read 0x3DA to reset
        // flip-flop; write index then data to 0x3C0; read data from 0x3C1.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        assert!(!v.atc_flip_flop_data);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL)); // Mode Control index, PAS=0
        assert!(v.atc_flip_flop_data);
        assert_eq!(v.atc_index, VGA_ATC_MODE_CONTROL);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_MODE_CONTROL_DEFAULT),
        );
        assert!(!v.atc_flip_flop_data);
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)],
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        // Read path: reset → write index → read 0x3C1 (does not toggle flip-flop).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        assert!(v.atc_flip_flop_data); // still awaiting data after address write
        assert_eq!(
            v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8,
            VGA_ATC_MODE_CONTROL
        );
    }

    #[test]
    fn attribute_controller_palette_and_mode_control_round_trip() {
        // Spec: FreeVGA Attribute Controller — palette indexes 0x00–0x0F and
        // Mode Control 0x10 are SeaBIOS-touchable ATC registers.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06); // palette brown entry
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x14);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x12); // Color Plane Enable
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x0F);
        assert_eq!(v.atc_regs[0x06], 0x14);
        assert_eq!(v.atc_regs[0x12], 0x0F);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x14);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x12);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0F);
    }

    #[test]
    fn attribute_controller_word_write_index_and_data() {
        // Consecutive lo/hi bytes through the flip-flop (index then data).
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 2, 0x0F_12);
        assert_eq!(v.atc_index, 0x12);
        assert_eq!(v.atc_regs[0x12], 0x0F);
        assert!(!v.atc_flip_flop_data);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x12);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0F);
    }

    #[test]
    fn attribute_controller_input_status1_resets_flip_flop() {
        // Spec: FreeVGA — mid-sequence read of 0x3DA forces address state so the
        // next 0x3C0 write is treated as index (not data).
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        // Flip-flop reset is independent of the status-bit model; bit values vary.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        assert!(!v.atc_flip_flop_data);
        // Without reset this would have been a data write to index 0x10.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x11); // Overscan index
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x55);
        assert_eq!(v.atc_regs[0x11], 0x55);
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)],
            VGA_ATC_MODE_CONTROL_DEFAULT
        ); // Mode Control default untouched
    }

    #[test]
    fn input_status1_vertical_retrace_and_display_disable_bits() {
        // Spec: FreeVGA External Registers / IBM VGA / OSDev VGA Hardware —
        // Input Status #1 (color `0x3DA`): bit0 = Display Disabled (inverted
        // display-enable; set during H or V retrace), bit3 = Vertical Retrace
        // (set during vertical retrace). Reading also resets the ATC flip-flop.
        let mut v = VgaText::new();
        assert_eq!(v.status1_phase, 0);

        let a = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(a & VGA_STATUS1_VR, 0, "phase0: not in vertical retrace");
        assert_eq!(a & VGA_STATUS1_DD, 0, "phase0: display enabled");
        assert!(!v.atc_flip_flop_data);

        let b = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_ne!(b & VGA_STATUS1_VR, 0, "phase1: vertical retrace");
        assert_ne!(b & VGA_STATUS1_DD, 0, "phase1: display disabled during VR");
        assert_eq!(
            b & (VGA_STATUS1_DD | VGA_STATUS1_VR),
            VGA_STATUS1_DD | VGA_STATUS1_VR
        );

        let c = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(c & VGA_STATUS1_VR, 0, "phase2: leaves vertical retrace");
        assert_eq!(c & VGA_STATUS1_DD, 0, "phase2: display enabled again");
    }

    #[test]
    fn input_status1_seabios_style_wait_for_retrace_terminates() {
        // Firmware commonly polls: wait until VR set, then wait until VR clear.
        // Deterministic read-phase model must make both waits terminate.
        let mut v = VgaText::new();
        let mut saw_vr = false;
        for _ in 0..8 {
            let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
            if s & VGA_STATUS1_VR != 0 {
                saw_vr = true;
                break;
            }
        }
        assert!(saw_vr, "must eventually observe Vertical Retrace");

        let mut left_vr = false;
        for _ in 0..8 {
            let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
            if s & VGA_STATUS1_VR == 0 {
                left_vr = true;
                break;
            }
        }
        assert!(left_vr, "must eventually leave Vertical Retrace");
    }

    #[test]
    fn input_status1_phase_advances_only_on_status_read() {
        let mut v = VgaText::new();
        let first = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        // ATC / Misc traffic must not advance the status phase.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        let _ = v.port_read(VGA_ATC_DATA_READ, 1);
        let _ = v.port_read(VGA_MISC_OUTPUT_READ, 1);
        assert_eq!(v.status1_phase, 1);
        let second = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_ne!(first & VGA_STATUS1_VR, second & VGA_STATUS1_VR);
    }

    #[test]
    fn input_status1_reset_clears_phase_and_keeps_flip_flop_reset() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1); // phase 0 → 1
        assert_eq!(v.status1_phase, 1);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1); // phase 1 → 0 (wraps)
        assert_eq!(v.status1_phase, 0);
        // Advance again so reset has a non-zero phase to clear.
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        assert_eq!(v.status1_phase, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        v.reset();
        assert_eq!(v.status1_phase, 0);
        assert!(!v.atc_flip_flop_data);
        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
        assert!(!v.atc_flip_flop_data);
    }

    #[test]
    fn input_status1_write_ignored() {
        let mut v = VgaText::new();
        v.port_write(VGA_INPUT_STATUS_1, 1, 0xFF);
        assert_eq!(v.status1_phase, 0);
        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
    }

    #[test]
    fn input_status1_mono_alias_when_misc_ioas_cleared() {
        // Spec: FreeVGA / IBM VGA Misc Output bit0 (IOAS) — clear selects mono
        // I/O map; Input Status #1 moves to `0x3BA` with the same DD/VR model
        // and ATC flip-flop reset. Color `0x3DA` is not owned.
        let mut v = VgaText::new();
        assert!(v.misc_ioas_color());
        assert!(v.owns_port(VGA_INPUT_STATUS_1));
        assert!(!v.owns_port(VGA_INPUT_STATUS_1_MONO));

        // Clear IOAS (keep other default-ish bits); mono status map.
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        assert!(!v.misc_ioas_color());
        assert!(!v.owns_port(VGA_INPUT_STATUS_1));
        assert!(v.owns_port(VGA_INPUT_STATUS_1_MONO));

        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);

        let a = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(a & VGA_STATUS1_VR, 0, "phase0: not in vertical retrace");
        assert_eq!(a & VGA_STATUS1_DD, 0, "phase0: display enabled");
        assert!(!v.atc_flip_flop_data);

        let b = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(
            b & (VGA_STATUS1_DD | VGA_STATUS1_VR),
            VGA_STATUS1_DD | VGA_STATUS1_VR
        );

        // Inactive color port must not advance phase or reset the flip-flop.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        let phase_before = v.status1_phase;
        assert_eq!(v.port_read(VGA_INPUT_STATUS_1, 1), 0xFFFF_FFFF);
        assert_eq!(v.status1_phase, phase_before);
        assert!(v.atc_flip_flop_data);
    }

    #[test]
    fn input_status1_color_port_when_misc_ioas_set() {
        // Spec: FreeVGA / IBM — IOAS=1 keeps Input Status #1 at `0x3DA`; mono
        // `0x3BA` is ignored / not owned.
        let mut v = VgaText::new();
        assert_eq!(v.misc_output & VGA_MISC_IOAS, VGA_MISC_IOAS);
        assert!(v.owns_port(VGA_INPUT_STATUS_1));
        assert!(!v.owns_port(VGA_INPUT_STATUS_1_MONO));

        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert!(v.atc_flip_flop_data);
        let phase_before = v.status1_phase;
        assert_eq!(v.port_read(VGA_INPUT_STATUS_1_MONO, 1), 0xFFFF_FFFF);
        assert_eq!(v.status1_phase, phase_before);
        assert!(v.atc_flip_flop_data);

        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.status1_phase, 1);
    }

    #[test]
    fn input_status1_ioas_switch_shares_phase_counter() {
        // Spec: FreeVGA External Registers — color/mono are aliases of one
        // Input Status #1; the phase model is shared across IOAS switches.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1); // phase 0 → 1
        assert_eq!(v.status1_phase, 1);

        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        let s = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(
            s & (VGA_STATUS1_DD | VGA_STATUS1_VR),
            VGA_STATUS1_DD | VGA_STATUS1_VR
        );
        assert_eq!(v.status1_phase, 0);

        // Restore color map; next read continues the shared counter.
        v.port_write(VGA_MISC_OUTPUT_WRITE, 1, u32::from(VGA_MISC_OUTPUT_DEFAULT));
        let s = v.port_read(VGA_INPUT_STATUS_1, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
        assert_eq!(v.status1_phase, 1);
    }

    #[test]
    fn input_status1_mono_write_ignored() {
        let mut v = VgaText::new();
        v.port_write(
            VGA_MISC_OUTPUT_WRITE,
            1,
            u32::from(VGA_MISC_OUTPUT_DEFAULT & !VGA_MISC_IOAS),
        );
        v.port_write(VGA_INPUT_STATUS_1_MONO, 1, 0xFF);
        assert_eq!(v.status1_phase, 0);
        let s = v.port_read(VGA_INPUT_STATUS_1_MONO, 1) as u8;
        assert_eq!(s & (VGA_STATUS1_DD | VGA_STATUS1_VR), 0);
    }

    #[test]
    fn attribute_controller_pas_bit_stored_in_address() {
        // Spec: FreeVGA Attribute Address — bit5 PAS; bits4:0 select register.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(
            VGA_ATC_ADDRESS_DATA,
            1,
            u32::from(VGA_ATC_PAS | VGA_ATC_MODE_CONTROL),
        );
        assert_eq!(v.atc_index, VGA_ATC_PAS | VGA_ATC_MODE_CONTROL);
        assert_eq!(
            v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8,
            VGA_ATC_PAS | VGA_ATC_MODE_CONTROL
        );
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        // Finishing the data write leaves PAS in the address register.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x08);
        assert_eq!(v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)], 0x08);
        assert_eq!(
            v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8,
            VGA_ATC_PAS | VGA_ATC_MODE_CONTROL
        );
    }

    /// Spec: FreeVGA — ATC address write retains PAS (bit5) in `atc_index`.
    #[test]
    fn atc_pas_bit5_retained_on_address_write() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1); // flip-flop → address
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_PAS | 0x05));
        assert_eq!(v.atc_index & VGA_ATC_PAS, VGA_ATC_PAS);
        assert_eq!(v.atc_index & 0x1F, 0x05);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x03); // PAS clear
        assert_eq!(v.atc_index & VGA_ATC_PAS, 0);
        assert_eq!(v.atc_index & 0x1F, 0x03);
    }

    #[test]
    fn attribute_controller_out_of_range_index_ignored_on_data() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x15); // beyond 0x00–0x14
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x55);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x15);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0);
    }

    #[test]
    fn attribute_controller_reset_defaults_mode03h() {
        // Spec: FreeVGA / IBM VGA / Abrash mode-03h-class ATC — palette
        // 00..05/14/07/38..3F, Mode Control [`VGA_ATC_MODE_CONTROL_DEFAULT`],
        // Color Plane Enable [`VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT`],
        // Horizontal PEL Panning [`VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT`],
        // Color Select [`VGA_ATC_COLOR_SELECT_DEFAULT`].
        let v = VgaText::new();
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        assert_eq!(v.atc_regs[0x06], 0x14);
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)],
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_PLANE_ENABLE)],
            VGA_ATC_COLOR_PLANE_ENABLE_DEFAULT
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_HORIZONTAL_PEL_PANNING)],
            VGA_ATC_HORIZONTAL_PEL_PANNING_DEFAULT
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)],
            VGA_ATC_COLOR_SELECT_DEFAULT
        );
    }

    #[test]
    fn reset_restores_attribute_controller_defaults() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x00);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        assert_eq!(v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)], 0x00);
        assert_eq!(v.atc_regs[0x06], 0x06);
        v.reset();
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        assert_eq!(
            v.port_read(VGA_ATC_DATA_READ, 1) as u8,
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x14);
    }

    #[test]
    fn dac_pel_owns_ports() {
        // Spec: FreeVGA Color Registers / OSDev VGA Hardware / RBIL — PEL Mask
        // `0x3C6`, DAC Read Index `0x3C7`, Write Index `0x3C8`, Data `0x3C9`.
        let v = VgaText::new();
        assert!(v.owns_port(VGA_DAC_PEL_MASK));
        assert!(v.owns_port(VGA_DAC_READ_INDEX));
        assert!(v.owns_port(VGA_DAC_WRITE_INDEX));
        assert!(v.owns_port(VGA_DAC_DATA));
    }

    #[test]
    fn dac_pel_mask_default_ff_and_readback() {
        // Spec: FreeVGA / RBIL / IBM VGA — PEL Mask at `0x3C6` is R/W; power-on
        // / reset default is `0xFF` (pass all color-index bits).
        let mut v = VgaText::new();
        assert_eq!(v.dac_pel_mask, VGA_DAC_PEL_MASK_DEFAULT);
        assert_eq!(
            v.port_read(VGA_DAC_PEL_MASK, 1) as u8,
            VGA_DAC_PEL_MASK_DEFAULT
        );
        v.port_write(VGA_DAC_PEL_MASK, 1, 0x0F);
        assert_eq!(v.port_read(VGA_DAC_PEL_MASK, 1) as u8, 0x0F);
        assert_eq!(v.dac_pel_mask, 0x0F);
        v.port_write(VGA_DAC_PEL_MASK, 1, 0xA5);
        assert_eq!(v.port_read(VGA_DAC_PEL_MASK, 1) as u8, 0xA5);
        v.reset();
        assert_eq!(v.dac_pel_mask, VGA_DAC_PEL_MASK_DEFAULT);
        assert_eq!(
            v.port_read(VGA_DAC_PEL_MASK, 1) as u8,
            VGA_DAC_PEL_MASK_DEFAULT
        );
    }

    #[test]
    fn dac_pel_mask_does_not_alter_dac_data_path() {
        // Spec: FreeVGA / RBIL / Abrash — PEL Mask ANDs the *displayed* pixel
        // color index before DAC lookup. It does not transform palette RAM
        // programming via `0x3C9`. Verify `0x3C9` store/readback is unchanged
        // under a non-`0xFF` mask (display AND is separate).
        let mut v = VgaText::new();
        v.port_write(VGA_DAC_PEL_MASK, 1, 0x00);
        v.port_write(VGA_DAC_WRITE_INDEX, 1, 0x10);
        v.port_write(VGA_DAC_DATA, 1, 0x3F);
        v.port_write(VGA_DAC_DATA, 1, 0x2A);
        v.port_write(VGA_DAC_DATA, 1, 0x15);
        assert_eq!(v.dac_ram[0x10], [0x3F, 0x2A, 0x15]);
        v.port_write(VGA_DAC_READ_INDEX, 1, 0x10);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x3F);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x2A);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x15);
        assert_eq!(v.port_read(VGA_DAC_PEL_MASK, 1) as u8, 0x00);
        // Display path sees mask 0x00 → index 0; DAC RAM entry 0x10 unchanged.
        assert_eq!(v.display_dac_index(0x10), 0x00);
        assert_eq!(v.display_dac_rgb(0x10), v.dac_ram[0]);
        assert_eq!(v.dac_ram[0x10], [0x3F, 0x2A, 0x15]);
    }

    /// Spec: FreeVGA Attribute Controller Internal Palette (indexes `0x00`–`0x0F`)
    /// — mode-03h defaults remap attribute color indices to DAC indexes
    /// (`00..05/14/07/38..3F`) before PEL Mask on the host text path.
    #[test]
    fn atc_internal_palette_default_remaps_host_text_attr_to_dac() {
        let v = VgaText::new();
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_P54S,
            0
        );
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)],
            VGA_ATC_COLOR_SELECT_DEFAULT
        );
        // Identity entries stay identity; brown (6) and intense colors remap.
        assert_eq!(v.atc_palette_dac_index(0x00), 0x00);
        assert_eq!(v.atc_palette_dac_index(0x05), 0x05);
        assert_eq!(v.atc_palette_dac_index(0x06), 0x14);
        assert_eq!(v.atc_palette_dac_index(0x07), 0x07);
        assert_eq!(v.atc_palette_dac_index(0x08), 0x38);
        assert_eq!(v.atc_palette_dac_index(0x0E), 0x3E);
        assert_eq!(v.atc_palette_dac_index(0x0F), 0x3F);
        // Attr 0x1E → fg 0x0E → ATC[0x0E]=0x3E; bg 0x01 → ATC[0x01]=0x01.
        assert_eq!(v.text_attr_fg_dac_index(0x1E), 0x3E);
        assert_eq!(v.text_attr_bg_dac_index(0x1E), 0x01);
        // Attr 0x16 → fg brown index 6 → DAC 0x14.
        assert_eq!(v.text_attr_fg_dac_index(0x16), 0x14);
    }

    /// Spec: FreeVGA Attribute Controller Mode Control `P54S` + Color Select —
    /// with P54S clear, Internal Palette bits 5:0 supply DAC address bits 5:0,
    /// while Color Select bits 3:2 always supply DAC address bits 7:6.
    #[test]
    fn atc_color_select_p54s_clear_preserves_internal_palette_bits_5_4() {
        let mut v = VgaText::new();
        v.atc_regs[0x0E] = 0x2A;
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_P54S,
            0
        );

        for (color_select, expected) in [(0x00, 0x2A), (0x05, 0x6A), (0x0A, 0xAA), (0x0F, 0xEA)] {
            v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)] = color_select;
            assert_eq!(v.atc_palette_dac_index(0x0E), expected);
        }
    }

    /// Spec: FreeVGA Attribute Controller Mode Control `P54S` + Color Select —
    /// with P54S set, Color Select bits 1:0 replace Internal Palette bits 5:4;
    /// Color Select bits 3:2 continue to supply DAC address bits 7:6.
    #[test]
    fn atc_color_select_p54s_set_replaces_internal_palette_bits_5_4() {
        let mut v = VgaText::new();
        v.atc_regs[0x0E] = 0x2A;
        v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] |= VGA_ATC_MODE_P54S;

        for (color_select, expected) in [(0x00, 0x0A), (0x05, 0x5A), (0x0A, 0xAA), (0x0F, 0xFA)] {
            v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)] = color_select;
            assert_eq!(v.atc_palette_dac_index(0x0E), expected);
        }
    }

    /// Spec: FreeVGA Attribute Controller + Color Registers — compose the
    /// eight-bit DAC address first, then AND that address with PEL Mask.
    #[test]
    fn atc_color_select_composes_before_pel_mask() {
        let mut v = VgaText::new();
        v.atc_regs[0x0E] = 0x2A;
        v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] |= VGA_ATC_MODE_P54S;
        v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)] = 0x0D;
        v.port_write(VGA_DAC_PEL_MASK, 1, 0x3F);

        assert_eq!(v.atc_palette_dac_index(0x0E), 0xDA);
        assert_eq!(v.text_attr_fg_dac_index(0x1E), 0x1A);
    }

    /// Spec: FreeVGA Attribute Controller — Color Select/P54S composition is
    /// shared by foreground, background, and the blink-off foreground path.
    #[test]
    fn atc_color_select_applies_to_text_foreground_background_and_blink_phase() {
        let mut v = VgaText::new();
        v.atc_regs[0x0E] = 0x2E;
        v.atc_regs[0x01] = 0x21;
        v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] |= VGA_ATC_MODE_P54S;
        v.atc_regs[usize::from(VGA_ATC_COLOR_SELECT)] = 0x0D;
        let attr = 0x9E;

        assert_eq!(v.text_attr_fg_dac_index(attr), 0xDE);
        assert_eq!(v.text_attr_bg_dac_index(attr), 0xD1);
        assert_eq!(v.text_attr_fg_dac_index_for_phase(attr, false), 0xDE);
        assert_eq!(v.text_attr_fg_dac_index_for_phase(attr, true), 0xD1);
    }

    /// Spec: FreeVGA — programming Internal Palette `0x00`–`0x0F` changes the
    /// host text attr→DAC remap; PEL Mask applies after the palette lookup.
    #[test]
    fn atc_internal_palette_program_remaps_then_pel_mask() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        // Remap attr color 0x0E → DAC 0x2A via ATC palette index 0x0E.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x0E);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x2A);
        assert_eq!(v.atc_regs[0x0E], 0x2A);
        assert_eq!(v.atc_palette_dac_index(0x0E), 0x2A);
        assert_eq!(v.text_attr_fg_dac_index(0x1E), 0x2A);

        v.port_write(VGA_DAC_PEL_MASK, 1, 0x0F);
        // 0x2A & 0x0F = 0x0A after remap.
        assert_eq!(v.text_attr_fg_dac_index(0x1E), 0x0A);
        // Raw display_dac_index stays PEL-mask-only (no ATC).
        assert_eq!(v.display_dac_index(0x2A), 0x0A);
    }

    /// Spec: FreeVGA — reset restores Internal Palette mode-03h defaults so
    /// host text attr→DAC helpers remap through `00..05/14/07/38..3F` again.
    #[test]
    fn atc_internal_palette_reset_restores_host_text_remap() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x11);
        assert_eq!(v.text_attr_fg_dac_index(0x16), 0x11);
        v.reset();
        assert_eq!(v.atc_regs[0x06], 0x14);
        assert_eq!(v.text_attr_fg_dac_index(0x16), 0x14);
        assert_eq!(v.text_attr_fg_dac_index(0x1E), 0x3E);
    }

    /// Spec: FreeVGA Attribute Controller Mode Control (index `0x10`) bit3
    /// (BLINK) — mode-03h reset default `0x0C` enables blink, so attribute bit7
    /// is blink enable and background uses bits 6:4 only (not intensity).
    #[test]
    fn atc_mode_control_default_blink_masks_attr_bit7_from_bg() {
        let v = VgaText::new();
        assert!(v.atc_blink_enabled());
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)] & VGA_ATC_MODE_BLINK,
            VGA_ATC_MODE_BLINK
        );
        // Attr 0x9E: bit7 set, bits 6:4 = 001 → bg ATC[1]=0x01 (not intensity 0x09).
        assert_eq!(v.text_attr_bg_dac_index(0x9E), 0x01);
        assert_eq!(v.text_attr_bg_dac_index(0x1E), 0x01);
        assert!(v.text_attr_blinks(0x9E));
        assert!(!v.text_attr_blinks(0x1E));
        // fg 0x0E → ATC[0x0E]=0x3E (mode-03h Internal Palette).
        assert_eq!(v.text_attr_fg_dac_index(0x9E), 0x3E);
    }

    /// Spec: FreeVGA — when Mode Control BLINK is clear, attribute bit7 selects
    /// background intensity (16 background colors via bits 7:4).
    #[test]
    fn atc_mode_control_blink_clear_uses_attr_bit7_as_bg_intensity() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        // LGE only (bit2); BLINK cleared.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x04);
        assert!(!v.atc_blink_enabled());
        // bg index 0x09 → ATC[0x09]=0x39; bg 0x01 → ATC[0x01]=0x01.
        assert_eq!(v.text_attr_bg_dac_index(0x9E), 0x39);
        assert_eq!(v.text_attr_bg_dac_index(0x1E), 0x01);
        assert!(!v.text_attr_blinks(0x9E));
        // Blink-off phase ignored when Mode Control blink is clear.
        assert_eq!(v.text_attr_fg_dac_index_for_phase(0x9E, true), 0x3E);
        assert_eq!(v.text_attr_fg_dac_index_for_phase(0x9E, false), 0x3E);
    }

    /// Spec: FreeVGA VGA Text Mode Operation — with blink enabled and attr bit7
    /// set, foreground alternates with background; blink-off half draws as bg.
    #[test]
    fn atc_mode_control_blink_off_half_draws_fg_as_bg() {
        let v = VgaText::new();
        assert!(v.atc_blink_enabled());
        let attr = 0x9E; // blink + bg bits 6:4 = 1, fg = 0x0E → DAC 0x3E / 0x01
        assert_eq!(v.text_attr_fg_dac_index_for_phase(attr, false), 0x3E);
        assert_eq!(v.text_attr_fg_dac_index_for_phase(attr, true), 0x01);
        // Non-blinking cell keeps remapped fg on both phases.
        assert_eq!(v.text_attr_fg_dac_index_for_phase(0x1E, true), 0x3E);
        assert_eq!(v.text_attr_fg_dac_index_for_phase(0x1E, false), 0x3E);
    }

    /// Spec: FreeVGA — reset restores Mode Control `0x0C` (BLINK|LGE) so host
    /// text helpers return to blink-enabled attribute interpretation.
    #[test]
    fn atc_mode_control_blink_reset_restores_default_interpretation() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_MODE_CONTROL));
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x00);
        assert!(!v.atc_blink_enabled());
        assert_eq!(v.text_attr_bg_dac_index(0x9E), 0x39);
        v.reset();
        assert!(v.atc_blink_enabled());
        assert_eq!(
            v.atc_regs[usize::from(VGA_ATC_MODE_CONTROL)],
            VGA_ATC_MODE_CONTROL_DEFAULT
        );
        assert_eq!(v.text_attr_bg_dac_index(0x9E), 0x01);
        assert!(v.text_attr_blinks(0x9E));
    }

    /// Spec: FreeVGA Color Registers / RBIL — PEL Mask default `0xFF` is an
    /// identity AND on the post-ATC display-path DAC index.
    #[test]
    fn dac_pel_mask_ff_identity_on_host_text_display_path() {
        let v = VgaText::new();
        assert_eq!(v.dac_pel_mask, VGA_DAC_PEL_MASK_DEFAULT);
        assert_eq!(v.display_dac_index(0x00), 0x00);
        assert_eq!(v.display_dac_index(0x0E), 0x0E);
        assert_eq!(v.display_dac_index(0xA5), 0xA5);
        // Attr 0x1E → fg ATC[0x0E]=0x3E / bg ATC[0x01]=0x01; mask 0xFF passes.
        assert_eq!(v.text_attr_fg_dac_index(0x1E), 0x3E);
        assert_eq!(v.text_attr_bg_dac_index(0x1E), 0x01);
        assert_eq!(v.display_dac_rgb(0x0E), VGA_DAC_CGA16_DEFAULTS[0x0E]);
        assert_eq!(v.display_dac_rgb(0x07), VGA_DAC_CGA16_DEFAULTS[0x07]);
    }

    /// Spec: FreeVGA — PEL Mask ANDs the post-ATC DAC index before DAC lookup.
    #[test]
    fn dac_pel_mask_restricts_host_text_display_dac_index() {
        let mut v = VgaText::new();
        // Distinct RGB at unmasked index 0x3E (ATC remap of fg 0x0E) and 0x06.
        v.port_write(VGA_DAC_WRITE_INDEX, 1, 0x3E);
        v.port_write(VGA_DAC_DATA, 1, 0x3F);
        v.port_write(VGA_DAC_DATA, 1, 0x00);
        v.port_write(VGA_DAC_DATA, 1, 0x00);
        v.port_write(VGA_DAC_WRITE_INDEX, 1, 0x06);
        v.port_write(VGA_DAC_DATA, 1, 0x00);
        v.port_write(VGA_DAC_DATA, 1, 0x3F);
        v.port_write(VGA_DAC_DATA, 1, 0x00);

        v.port_write(VGA_DAC_PEL_MASK, 1, 0x07);
        assert_eq!(v.display_dac_index(0x0E), 0x06);
        // fg 0x0E → ATC 0x3E → & 0x07 = 0x06.
        assert_eq!(v.text_attr_fg_dac_index(0x1E), 0x06);
        // Default Mode Control blink → bg bits 6:4 = 0x01 → ATC 0x01 & mask.
        assert_eq!(v.text_attr_bg_dac_index(0x9E), 0x01);
        assert_eq!(v.display_dac_rgb(0x3E), [0x00, 0x3F, 0x00]);
        // Programming path still sees unmasked RAM entries.
        assert_eq!(v.dac_ram[0x3E], [0x3F, 0x00, 0x00]);
        assert_eq!(v.dac_ram[0x06], [0x00, 0x3F, 0x00]);
    }

    /// Spec: FreeVGA / IBM VGA — reset restores PEL Mask `0xFF` so host display
    /// helpers return to identity AND after Internal Palette remap.
    #[test]
    fn dac_pel_mask_reset_restores_display_path_identity() {
        let mut v = VgaText::new();
        v.port_write(VGA_DAC_PEL_MASK, 1, 0x0F);
        assert_eq!(v.display_dac_index(0xA5), 0x05);
        // Attr fg 0x0C → ATC[0x0C]=0x3C → & 0x0F = 0x0C.
        assert_eq!(v.text_attr_fg_dac_index(0x3C), 0x0C);
        v.reset();
        assert_eq!(v.dac_pel_mask, VGA_DAC_PEL_MASK_DEFAULT);
        assert_eq!(v.display_dac_index(0xA5), 0xA5);
        assert_eq!(v.text_attr_fg_dac_index(0x3C), 0x3C);
        assert_eq!(v.display_dac_rgb(0x0F), VGA_DAC_CGA16_DEFAULTS[0x0F]);
    }

    #[test]
    fn dac_pel_write_index_rgb_round_trip() {
        // Spec: FreeVGA Color Registers / IBM VGA — write index at 0x3C8, then
        // R→G→B to 0x3C9; read index at 0x3C7, then R→G→B from 0x3C9.
        let mut v = VgaText::new();
        v.port_write(VGA_DAC_WRITE_INDEX, 1, 0x10);
        assert_eq!(v.port_read(VGA_DAC_WRITE_INDEX, 1) as u8, 0x10);
        assert_eq!(
            v.port_read(VGA_DAC_READ_INDEX, 1) as u8,
            VGA_DAC_STATE_WRITE
        );
        v.port_write(VGA_DAC_DATA, 1, 0x3F);
        v.port_write(VGA_DAC_DATA, 1, 0x2A);
        v.port_write(VGA_DAC_DATA, 1, 0x15);
        assert_eq!(v.dac_ram[0x10], [0x3F, 0x2A, 0x15]);
        // Auto-increment after blue.
        assert_eq!(v.dac_write_index, 0x11);
        assert_eq!(v.dac_write_channel, 0);

        v.port_write(VGA_DAC_READ_INDEX, 1, 0x10);
        assert_eq!(v.port_read(VGA_DAC_READ_INDEX, 1) as u8, VGA_DAC_STATE_READ);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x3F);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x2A);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x15);
        assert_eq!(v.dac_read_index, 0x11);
        assert_eq!(v.dac_read_channel, 0);
    }

    #[test]
    fn dac_pel_auto_increment_consecutive_entries() {
        // Spec: FreeVGA DAC Operation — after each RGB triplet the index
        // advances so the next entry can be programmed without reloading.
        let mut v = VgaText::new();
        v.port_write(VGA_DAC_WRITE_INDEX, 1, 0x20);
        v.port_write(VGA_DAC_DATA, 1, 0x01);
        v.port_write(VGA_DAC_DATA, 1, 0x02);
        v.port_write(VGA_DAC_DATA, 1, 0x03);
        v.port_write(VGA_DAC_DATA, 1, 0x04);
        v.port_write(VGA_DAC_DATA, 1, 0x05);
        v.port_write(VGA_DAC_DATA, 1, 0x06);
        assert_eq!(v.dac_ram[0x20], [0x01, 0x02, 0x03]);
        assert_eq!(v.dac_ram[0x21], [0x04, 0x05, 0x06]);
        assert_eq!(v.port_read(VGA_DAC_WRITE_INDEX, 1) as u8, 0x22);

        v.port_write(VGA_DAC_READ_INDEX, 1, 0x20);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x01);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x02);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x03);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x04);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x05);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x06);
    }

    #[test]
    fn dac_pel_masks_to_six_bits() {
        // Spec: FreeVGA Color Registers / IBM VGA — DAC data is 6-bit (5:0).
        let mut v = VgaText::new();
        v.port_write(VGA_DAC_WRITE_INDEX, 1, 0x05);
        v.port_write(VGA_DAC_DATA, 1, 0xFF);
        v.port_write(VGA_DAC_DATA, 1, 0xC0);
        v.port_write(VGA_DAC_DATA, 1, 0x7E);
        assert_eq!(v.dac_ram[0x05], [0x3F, 0x00, 0x3E]);
        v.port_write(VGA_DAC_READ_INDEX, 1, 0x05);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x3F);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x00);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x3E);
    }

    #[test]
    fn dac_pel_reset_defaults_mode03h() {
        // Spec: IBM VGA / classic CGA–EGA 16-color 6-bit palette for indices
        // 0–15; remaining entries black. Store/readback only.
        let v = VgaText::new();
        assert_eq!(&v.dac_ram[..16], &VGA_DAC_CGA16_DEFAULTS);
        assert_eq!(v.dac_ram[0], [0x00, 0x00, 0x00]);
        assert_eq!(v.dac_ram[7], [0x2A, 0x2A, 0x2A]);
        assert_eq!(v.dac_ram[15], [0x3F, 0x3F, 0x3F]);
        assert_eq!(v.dac_ram[16], [0x00, 0x00, 0x00]);
        assert_eq!(v.dac_ram[255], [0x00, 0x00, 0x00]);
        assert_eq!(v.dac_state, VGA_DAC_STATE_WRITE);

        // Readback path for a default entry.
        let mut v = VgaText::new();
        v.port_write(VGA_DAC_READ_INDEX, 1, 0x0E); // yellow
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x3F);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x3F);
        assert_eq!(v.port_read(VGA_DAC_DATA, 1) as u8, 0x15);
    }

    #[test]
    fn reset_restores_dac_pel_defaults() {
        let mut v = VgaText::new();
        v.port_write(VGA_DAC_PEL_MASK, 1, 0x55);
        v.port_write(VGA_DAC_WRITE_INDEX, 1, 0x07);
        v.port_write(VGA_DAC_DATA, 1, 0x11);
        v.port_write(VGA_DAC_DATA, 1, 0x22);
        v.port_write(VGA_DAC_DATA, 1, 0x33);
        v.port_write(VGA_DAC_READ_INDEX, 1, 0x40);
        assert_eq!(v.dac_ram[0x07], [0x11, 0x22, 0x33]);
        assert_eq!(v.dac_state, VGA_DAC_STATE_READ);
        assert_eq!(v.dac_pel_mask, 0x55);
        v.reset();
        assert_eq!(v.dac_pel_mask, VGA_DAC_PEL_MASK_DEFAULT);
        assert_eq!(v.dac_ram, vga_dac_default_ram());
        assert_eq!(v.dac_ram[0x07], VGA_DAC_CGA16_DEFAULTS[0x07]);
        assert_eq!(v.dac_write_index, 0);
        assert_eq!(v.dac_read_index, 0);
        assert_eq!(v.dac_write_channel, 0);
        assert_eq!(v.dac_read_channel, 0);
        assert_eq!(v.dac_state, VGA_DAC_STATE_WRITE);
    }

    /// Program Sequencer Memory Mode (`0x3C4`/`0x3C5` index `0x04`).
    fn set_seq_memory_mode(v: &mut VgaText, value: u8) {
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MEMORY_MODE));
        v.port_write(VGA_SEQ_DATA, 1, u32::from(value));
    }

    /// Program Sequencer Map Mask (`0x3C4`/`0x3C5` index `0x02`).
    fn set_seq_map_mask(v: &mut VgaText, value: u8) {
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(VGA_SEQ_MAP_MASK));
        v.port_write(VGA_SEQ_DATA, 1, u32::from(value));
    }

    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 — mode-03h Memory Mode
    /// `0x02` means Extended Memory set, Odd/Even addressing on, Chain 4 off;
    /// Figure 2-29 — mode-03h Map Mask `0x03` enables maps 0 and 1.
    #[test]
    fn seq_memory_mode_reset_defaults_select_odd_even_addressing() {
        let v = VgaText::new();
        assert!(v.seq_extended_memory());
        assert!(v.seq_odd_even_enabled());
        assert!(!v.seq_chain4_enabled());
        assert_eq!(v.seq_map_mask(), VGA_SEQ_MAP_MASK_DEFAULT);
        assert_eq!(v.plane_addressing(), VgaPlaneAddressing::OddEven);
        assert_eq!(v.plane_size_bytes(), VGA_PLANE_SIZE);
    }

    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 — "even system addresses
    /// access maps 0 and 2, while odd system addresses access maps 1 and 3";
    /// OSDev VGA Hardware "Odd/Even Disable Bit" — `offset = addr & 0xfffe`.
    /// The Map Mask then narrows mode 03h to maps 0 (character) and 1 (attribute).
    #[test]
    fn plane_access_odd_even_maps_even_addresses_to_maps_0_and_2() {
        let v = VgaText::new();

        let even = v.plane_access(VGA_TEXT_BASE).expect("in window");
        assert_eq!(even.addressing, VgaPlaneAddressing::OddEven);
        assert_eq!(
            even.planes, 0b0101,
            "even host address selects maps 0 and 2"
        );
        assert_eq!(even.write_planes, 0b0001, "Map Mask 0x03 keeps map 0");
        assert_eq!(even.offset, 0);

        let odd = v.plane_access(VGA_TEXT_BASE + 1).expect("in window");
        assert_eq!(odd.planes, 0b1010, "odd host address selects maps 1 and 3");
        assert_eq!(odd.write_planes, 0b0010, "Map Mask 0x03 keeps map 1");
        assert_eq!(odd.offset, 0, "A0 is dropped from the map offset");

        let cell1_char = v.plane_access(VGA_TEXT_BASE + 2).expect("in window");
        assert_eq!(cell1_char.planes, 0b0101);
        assert_eq!(cell1_char.offset, 2);
        let cell1_attr = v.plane_access(VGA_TEXT_BASE + 3).expect("in window");
        assert_eq!(cell1_attr.planes, 0b1010);
        assert_eq!(cell1_attr.offset, 2);

        // Map Mask widening exposes the chained upper maps.
        let mut v = v;
        set_seq_map_mask(&mut v, VGA_SEQ_MAP_MASK_PLANES);
        assert_eq!(v.plane_write_mask(VGA_TEXT_BASE), 0b0101);
        assert_eq!(v.plane_write_mask(VGA_TEXT_BASE + 1), 0b1010);
    }

    /// Spec: IBM PS/2 Video Subsystems Figure 2-34 "Map Selection, Chain 4" —
    /// A1 A0 select maps 0–3; OSDev VGA Hardware records the hardware offset
    /// form as the host address with the two low bits cleared.
    #[test]
    fn plane_access_chain4_selects_map_by_two_low_address_bits() {
        let mut v = VgaText::new();
        set_seq_memory_mode(
            &mut v,
            VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_CHAIN4,
        );
        set_seq_map_mask(&mut v, VGA_SEQ_MAP_MASK_PLANES);
        assert_eq!(v.plane_addressing(), VgaPlaneAddressing::Chain4);

        for (i, expected_plane) in [0b0001u8, 0b0010, 0b0100, 0b1000].iter().enumerate() {
            let access = v.plane_access(VGA_TEXT_BASE + i as u64).expect("in window");
            assert_eq!(access.planes, *expected_plane, "A1:A0 = {i}");
            assert_eq!(access.write_planes, *expected_plane);
            assert_eq!(access.offset, 0, "A1:A0 do not contribute to the offset");
        }
        let next = v.plane_access(VGA_TEXT_BASE + 4).expect("in window");
        assert_eq!(next.planes, 0b0001);
        assert_eq!(next.offset, 4);

        // Chain 4 still runs through the Map Mask (IBM: all maps should be
        // enabled in chain 4; OSDev reports plane write enable applies on
        // QEMU/ATI/NVidia).
        set_seq_map_mask(&mut v, 0b0011);
        assert_eq!(v.plane_write_mask(VGA_TEXT_BASE + 2), 0);
        assert_eq!(v.plane_write_mask(VGA_TEXT_BASE + 1), 0b0010);
    }

    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 — with OE = 1 "system
    /// addresses sequentially access data within a bit map, and the maps are
    /// accessed according to the value in the Map Mask register".
    #[test]
    fn plane_access_planar_addresses_all_maps_at_one_offset() {
        let mut v = VgaText::new();
        // Planar host addressing also needs GC Miscellaneous Chain Odd/Even
        // clear (IBM Figure 2-74 OE); keep Memory Map Select on `0xB8000`.
        set_gc_reg(
            &mut v,
            VGA_GC_MISC,
            VGA_GC_MISC_GRAPHICS_MODE | VGA_GC_MISC_MEMORY_MAP_MASK,
        );
        set_seq_memory_mode(
            &mut v,
            VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
        );
        set_seq_map_mask(&mut v, 0b1001);
        assert_eq!(v.plane_addressing(), VgaPlaneAddressing::Planar);

        let access = v.plane_access(VGA_TEXT_BASE + 0x123).expect("in window");
        assert_eq!(access.planes, VGA_SEQ_MAP_MASK_PLANES);
        assert_eq!(access.write_planes, 0b1001);
        assert_eq!(access.offset, 0x123, "planar offsets are not shifted");
        assert_eq!(v.plane_offset(VGA_TEXT_BASE + 0x124), Some(0x124));
    }

    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 — Extended Memory clear
    /// leaves 64 KB of video memory (16 KB per map). Wrapping the per-map
    /// offset inside that region is this emulator's documented model choice.
    #[test]
    fn plane_access_without_extended_memory_wraps_offset_in_16k_map() {
        let mut v = VgaText::new();
        set_seq_memory_mode(&mut v, VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE);
        assert!(!v.seq_extended_memory());
        assert_eq!(v.plane_size_bytes(), VGA_PLANE_SIZE_NO_EXTENDED);
        assert_eq!(v.plane_offset(VGA_TEXT_BASE + 0x0004), Some(0x0004));
        assert_eq!(v.plane_offset(VGA_TEXT_BASE + 0x4004), Some(0x0004));

        set_seq_memory_mode(
            &mut v,
            VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE | VGA_SEQ_MEMORY_MODE_EXTENDED,
        );
        assert_eq!(v.plane_offset(VGA_TEXT_BASE + 0x4004), Some(0x4004));
    }

    /// Addresses outside the CPU display window have no map mapping.
    #[test]
    fn plane_access_outside_display_window_is_none() {
        let v = VgaText::new();
        assert_eq!(v.display_window(), (VGA_TEXT_BASE, VGA_TEXT_END));
        assert!(v.plane_access(VGA_TEXT_BASE - 1).is_none());
        assert!(v.plane_access(VGA_TEXT_END).is_none());
        assert_eq!(v.plane_write_mask(VGA_TEXT_END), 0);
        assert!(v.plane_offset(VGA_TEXT_BASE - 1).is_none());
        assert!(v.plane_access(VGA_TEXT_END - 1).is_some());
    }

    /// The host alphanumeric helpers address the character/attribute
    /// interleave in display memory regardless of the Sequencer addressing
    /// mode, so a chain-4 programming does not disturb them.
    #[test]
    fn plane_addressing_does_not_disturb_text_plane_mmio() {
        let mut v = VgaText::new();
        set_seq_memory_mode(
            &mut v,
            VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_CHAIN4,
        );
        assert!(v.write_u8(VGA_TEXT_BASE, b'A'));
        assert!(v.write_u8(VGA_TEXT_BASE + 1, 0x1F));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'A'));
        assert_eq!(v.char_at(0, 0), Some(b'A'));
        assert_eq!(v.attr_at(0, 0), Some(0x1F));
    }

    /// Program a Graphics Controller register (`0x3CE`/`0x3CF`).
    fn set_gc_reg(v: &mut VgaText, index: u8, value: u8) {
        v.port_write(VGA_GC_INDEX, 1, u32::from(index));
        v.port_write(VGA_GC_DATA, 1, u32::from(value));
    }

    /// Spec: IBM PS/2 Video Subsystems Figures 2-68 / 2-71 / 2-76 — Color
    /// Compare, Read Map Select and Color Don't Care all reset to `0x00` in the
    /// mode-03h-class register file.
    #[test]
    fn gc_read_path_registers_reset_to_mode03h_defaults() {
        let v = VgaText::new();
        assert_eq!(v.gc_color_compare(), 0x00);
        assert_eq!(v.gc_read_map_select(), 0);
        assert_eq!(v.gc_color_dont_care(), 0x00);
        assert_eq!(v.gc_write_mode(), 0);
        assert_eq!(v.gc_read_mode(), 0);
        assert_eq!(v.gc_bit_mask(), VGA_GC_BIT_MASK_DEFAULT);
        assert_eq!(v.gc_rotate_count(), 0);
        assert_eq!(v.gc_function_select(), VGA_GC_FUNCTION_REPLACE);
    }

    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 + Figure 2-29 — under the
    /// mode-03h odd/even + Map Mask `0x03` programming the GC write path
    /// reaches the character map on even addresses and the attribute map on odd
    /// addresses, at one shared map offset.
    #[test]
    fn gc_write_odd_even_reaches_character_and_attribute_maps() {
        let mut v = VgaText::new();
        assert!(v.gc_write_u8(VGA_TEXT_BASE, b'Z'));
        assert!(v.gc_write_u8(VGA_TEXT_BASE + 1, 0x1F));
        assert_eq!(v.plane_byte(0, 0), Some(b'Z'));
        assert_eq!(v.plane_byte(1, 0), Some(0x1F));
        assert_eq!(v.plane_byte(2, 0), Some(0), "Map Mask 0x03 blocks map 2");
        assert_eq!(v.plane_byte(3, 0), Some(0), "Map Mask 0x03 blocks map 3");

        // Mode-03h leaves Graphics Mode bit4 Host Odd/Even read addressing set,
        // so A0 replaces Read Map Select bit0: even → character map, odd →
        // attribute map. Spec: FreeVGA Graphics Mode "Host O/E".
        set_gc_reg(&mut v, VGA_GC_READ_MAP_SELECT, 0);
        assert_eq!(v.gc_read_u8(VGA_TEXT_BASE), Some(b'Z'));
        assert_eq!(v.gc_read_u8(VGA_TEXT_BASE + 1), Some(0x1F));
        assert_eq!(v.gc_latches, [b'Z', 0x1F, 0, 0]);

        // With the bit clear, Read Map Select alone chooses the map.
        set_gc_reg(&mut v, VGA_GC_MODE, 0x00);
        set_gc_reg(&mut v, VGA_GC_READ_MAP_SELECT, 1);
        assert_eq!(v.gc_read_u8(VGA_TEXT_BASE), Some(0x1F));
    }

    /// Direct map helpers reject out-of-range maps and offsets.
    #[test]
    fn plane_byte_helpers_bound_map_and_offset() {
        let mut v = VgaText::new();
        assert!(v.set_plane_byte(3, VGA_PLANE_SIZE - 1, 0x7E));
        assert_eq!(v.plane_byte(3, VGA_PLANE_SIZE - 1), Some(0x7E));
        assert!(!v.set_plane_byte(VGA_PLANE_COUNT, 0, 1));
        assert!(!v.set_plane_byte(0, VGA_PLANE_SIZE, 1));
        assert_eq!(v.plane_byte(VGA_PLANE_COUNT, 0), None);
        assert_eq!(v.plane_byte(0, VGA_PLANE_SIZE), None);
    }

    /// Reset restores mode-03h Memory Mode / Map Mask, hence odd/even decode.
    #[test]
    fn reset_restores_plane_addressing_defaults() {
        let mut v = VgaText::new();
        set_seq_memory_mode(&mut v, VGA_SEQ_MEMORY_MODE_CHAIN4);
        set_seq_map_mask(&mut v, VGA_SEQ_MAP_MASK_PLANES);
        assert_eq!(v.plane_addressing(), VgaPlaneAddressing::Chain4);
        assert!(!v.seq_extended_memory());

        v.reset();
        assert_eq!(v.plane_addressing(), VgaPlaneAddressing::OddEven);
        assert_eq!(v.seq_map_mask(), VGA_SEQ_MAP_MASK_DEFAULT);
        assert!(v.seq_extended_memory());
        assert_eq!(v.plane_write_mask(VGA_TEXT_BASE), 0b0001);
    }
}

/// Character generator and text-mode display fetch (M2 round 3, slice 1).
///
/// Every expected pixel value here is computed from the specification —
/// FreeVGA "VGA Text Mode Operation" for the fetch, the FreeVGA Sequencer /
/// CRTC / Attribute Controller register pages for the fields, and the
/// mode-03h reset defaults this device already asserts elsewhere — not read
/// back out of the renderer.
///
/// These live beside the device rather than in `crates/devices/tests/` because
/// `crates/devices/src/lib.rs` does not yet re-export [`VgaFrame`],
/// [`VgaRenderMode`] or the new constants; see `docs/vga-r3-character-generator.md`.
#[cfg(test)]
mod character_generator_tests {
    use super::*;

    /// Mode-03h default ATC Internal Palette, from [`VGA_ATC_DEFAULTS`].
    const PALETTE: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E,
        0x3F,
    ];

    fn set_crtc(v: &mut VgaText, index: u8, value: u8) {
        v.port_write(VGA_CRTC_INDEX, 1, u32::from(index));
        v.port_write(VGA_CRTC_DATA, 1, u32::from(value));
    }

    fn set_seq(v: &mut VgaText, index: u8, value: u8) {
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(index));
        v.port_write(VGA_SEQ_DATA, 1, u32::from(value));
    }

    fn set_atc(v: &mut VgaText, index: u8, value: u8) {
        // Spec: FreeVGA Attribute Controller — `0x3C0` alternates address and
        // data; a status read resets the flip-flop to the address state.
        v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(index));
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(value));
    }

    fn set_gc(v: &mut VgaText, index: u8, value: u8) {
        v.port_write(VGA_GC_INDEX, 1, u32::from(index));
        v.port_write(VGA_GC_DATA, 1, u32::from(value));
    }

    /// Spec: FreeVGA CRT Controller Cursor Start Register bit5 `CD`.
    fn disable_cursor(v: &mut VgaText) {
        set_crtc(v, VGA_CRTC_CURSOR_START, VGA_CRTC_CURSOR_DISABLE);
    }

    /// Store a character/attribute pair at CRTC address-counter value `counter`.
    fn put_cell(v: &mut VgaText, counter: usize, code: u8, attr: u8) {
        let offset = v.display_map_offset(counter);
        assert!(v.set_plane_byte(VGA_TEXT_CHAR_PLANE, offset, code));
        assert!(v.set_plane_byte(VGA_TEXT_ATTR_PLANE, offset, attr));
    }

    /// Store glyph scan lines for `code` in the font bank at `bank`.
    fn put_glyph(v: &mut VgaText, bank: usize, code: u8, rows: &[u8]) {
        for (scan, byte) in rows.iter().enumerate() {
            let offset = bank + usize::from(code) * VGA_FONT_GLYPH_BYTES + scan;
            assert!(v.set_plane_byte(VGA_FONT_PLANE, offset, *byte));
        }
    }

    /// Spec: FreeVGA Sequencer Character Map Select Register — the field is
    /// deliberately non-contiguous for EGA compatibility.
    #[test]
    fn char_map_bank_offsets_follow_the_documented_table() {
        let expected = [
            0x0000usize,
            0x4000,
            0x8000,
            0xC000,
            0x2000,
            0x6000,
            0xA000,
            0xE000,
        ];
        for (select, want) in expected.iter().enumerate() {
            assert_eq!(
                VgaText::char_map_bank_offset(select as u8),
                *want,
                "select {select:03b}"
            );
        }
    }

    /// Spec: FreeVGA Sequencer Character Map Select — bit5 and bits 3:2 form
    /// Character Set A Select; bit4 and bits 1:0 form Character Set B Select.
    #[test]
    fn char_map_select_field_bits_are_split_the_documented_way() {
        let mut v = VgaText::new();
        assert_eq!(v.seq_char_map_a_select(), 0);
        assert_eq!(v.seq_char_map_b_select(), 0);

        // A = 111b (bit5 + bits3:2), B = 000b.
        set_seq(&mut v, VGA_SEQ_CHAR_MAP_SELECT, 0b0010_1100);
        assert_eq!(v.seq_char_map_a_select(), 0b111);
        assert_eq!(v.seq_char_map_b_select(), 0b000);

        // A = 000b, B = 111b (bit4 + bits1:0).
        set_seq(&mut v, VGA_SEQ_CHAR_MAP_SELECT, 0b0001_0011);
        assert_eq!(v.seq_char_map_a_select(), 0b000);
        assert_eq!(v.seq_char_map_b_select(), 0b111);
        assert_eq!(
            VGA_SEQ_CHAR_MAP_A_MASK | VGA_SEQ_CHAR_MAP_B_MASK,
            0b0011_1111
        );
    }

    /// Spec: FreeVGA Clocking Mode bit0 (9 dots when clear) + CRTC Maximum Scan
    /// Line ("character height - 1"). Mode-03h defaults are `0x00` and `0x0F`,
    /// so cells are 9×16 and the 80×25 grid is 720×400 pixels.
    #[test]
    fn mode03h_defaults_give_9x16_cells_and_a_720x400_frame() {
        let v = VgaText::new();
        assert_eq!(v.text_cell_width(), 9);
        assert_eq!(v.text_cell_height(), 16);
        assert_eq!(v.render_mode(), VgaRenderMode::Text);

        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.width, 720);
        assert_eq!(frame.height, 400);
        assert_eq!(frame.pixels.len(), 720 * 400);
        assert_eq!(frame.mode, VgaRenderMode::Text);
    }

    /// Reset installs no font, so no glyph is displayed until software loads
    /// one. There is no built-in character ROM here.
    ///
    /// The reset fill is 80×25 spaces with attribute `0x07`, so every glyph
    /// pixel is background (Internal Palette entry 0). The one lit region is
    /// the hardware cursor: the reset CRTC register file leaves Cursor
    /// Location `0x0000`, Cursor Start `0` and Cursor End `0` with Cursor
    /// Disable clear, so scan line 0 of the top-left cell is drawn in that
    /// cell's foreground. Spec: FreeVGA CRTC Cursor Start / End / Location.
    #[test]
    fn reset_leaves_no_font_so_only_the_hardware_cursor_is_lit() {
        let mut v = VgaText::new();
        let frame = v.render_frame(false).expect("text mode renders");
        for dot in 0..9 {
            assert_eq!(frame.index_at(dot, 0), Some(PALETTE[7]), "cursor dot {dot}");
        }
        assert_eq!(frame.index_at(9, 0), Some(PALETTE[0]));
        assert!((1..400).all(|y| (0..720).all(|x| frame.index_at(x, y) == Some(PALETTE[0]))));

        disable_cursor(&mut v);
        let frame = v.render_frame(false).expect("text mode renders");
        assert!(frame.pixels.iter().all(|p| *p == PALETTE[0]));
    }

    /// Spec: FreeVGA CRTC Mode Control bit6 / Underline Location `DW` — the
    /// display-side address multiplier. Word mode (the mode-03h default) puts
    /// counter value *n* at map offset `2n`, matching the CPU-side odd/even
    /// offset form.
    #[test]
    fn display_map_offset_follows_word_byte_and_doubleword_addressing() {
        let mut v = VgaText::new();
        assert!(!v.crtc_byte_addressing());
        assert!(!v.crtc_doubleword_addressing());
        assert_eq!(v.crtc_address_multiplier(), 2);
        assert_eq!(v.display_map_offset(0), 0);
        assert_eq!(v.display_map_offset(1), 2);
        assert_eq!(v.display_map_offset(80), 160);

        set_crtc(&mut v, VGA_CRTC_MODE_CONTROL, VGA_CRTC_MODE_BYTE_ADDRESSING);
        assert!(v.crtc_byte_addressing());
        assert_eq!(v.crtc_address_multiplier(), 1);
        assert_eq!(v.display_map_offset(80), 80);

        set_crtc(
            &mut v,
            VGA_CRTC_UNDERLINE_LOCATION,
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT | VGA_CRTC_UNDERLINE_DW,
        );
        assert!(v.crtc_doubleword_addressing());
        assert_eq!(v.crtc_address_multiplier(), 4);
        assert_eq!(v.display_map_offset(80), 320);

        // The counter is 16 bits and the map offset wraps inside the enabled
        // map size (64 KiB with Extended Memory set).
        set_crtc(&mut v, VGA_CRTC_UNDERLINE_LOCATION, 0x00);
        set_crtc(&mut v, VGA_CRTC_MODE_CONTROL, 0x00);
        assert_eq!(v.display_map_offset(0x1_0000), 0);
        assert_eq!(v.display_map_offset(0x8000), 0);
    }

    /// The fetch reads the code from map 0, the attribute from map 1, and the
    /// glyph from map 2 — and the resulting pixels are the ATC-composed DAC
    /// indices for that attribute.
    ///
    /// Spec: FreeVGA "VGA Text Mode Operation", Display Memory Organization +
    /// Fonts + Attributes.
    #[test]
    fn text_fetch_takes_code_attribute_and_glyph_from_maps_0_1_and_2() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        // Attribute 0x1E: foreground index 14, background index 1. With the
        // mode-03h Internal Palette, P54S clear and Color Select 0, the DAC
        // indices are the palette entries themselves.
        let attr = 0x1E;
        let fg = PALETTE[14];
        let bg = PALETTE[1];
        assert_eq!(v.text_attr_fg_dac_index(attr), fg);
        assert_eq!(v.text_attr_bg_dac_index(attr), bg);

        put_cell(&mut v, 0, b'A', attr);
        put_glyph(&mut v, 0, b'A', &[0b1000_0001, 0b0011_1100]);

        let frame = v.render_frame(false).expect("text mode renders");
        // Scan line 0: dots 0 and 7 lit, dot 8 (the ninth) is background
        // because code 0x41 is outside the C0h-DFh line-graphics range.
        let want_row0 = [fg, bg, bg, bg, bg, bg, bg, fg, bg];
        for (dot, want) in want_row0.iter().enumerate() {
            assert_eq!(frame.index_at(dot, 0), Some(*want), "row 0 dot {dot}");
        }
        let want_row1 = [bg, bg, fg, fg, fg, fg, bg, bg, bg];
        for (dot, want) in want_row1.iter().enumerate() {
            assert_eq!(frame.index_at(dot, 1), Some(*want), "row 1 dot {dot}");
        }
        // Rows with no glyph bits set are all background.
        assert!((2..16).all(|y| (0..9).all(|x| frame.index_at(x, y) == Some(bg))));
        // The neighbouring cell was never written, so it is code 0 / attr 0.
        assert_eq!(frame.index_at(9, 0), Some(PALETTE[0]));
    }

    /// A byte the CPU writes through the Graphics Controller in the mode-03h
    /// odd/even programming is the byte the character generator fetches.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 (odd/even) + Figure 2-29
    /// (Map Mask `0x03`) on the CPU side; FreeVGA CRTC Mode Control word mode
    /// on the display side.
    #[test]
    fn a_cpu_odd_even_write_is_what_the_character_generator_fetches() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        put_glyph(&mut v, 0, b'H', &[0xFF]);

        assert!(v.gc_write_u8(VGA_TEXT_BASE, b'H'));
        assert!(v.gc_write_u8(VGA_TEXT_BASE + 1, 0x1E));
        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 0), Some(b'H'));
        assert_eq!(v.plane_byte(VGA_TEXT_ATTR_PLANE, 0), Some(0x1E));

        let frame = v.render_frame(false).expect("text mode renders");
        for dot in 0..8 {
            assert_eq!(frame.index_at(dot, 0), Some(PALETTE[14]), "dot {dot}");
        }
        assert_eq!(frame.index_at(8, 0), Some(PALETTE[1]), "ninth dot");
    }

    /// Spec: FreeVGA Fonts — attribute bit 3 set selects Character Set A,
    /// clear selects Character Set B.
    #[test]
    fn attribute_bit3_selects_character_set_a_or_b() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        // Character Set A = 001b (bank 0x4000), Character Set B = 000b.
        set_seq(&mut v, VGA_SEQ_CHAR_MAP_SELECT, 0b0000_0100);
        assert_eq!(v.seq_char_map_a_select(), 0b001);
        assert_eq!(v.seq_char_map_b_select(), 0b000);
        assert_eq!(v.text_font_bank_offset(0x0F), 0x4000);
        assert_eq!(v.text_font_bank_offset(0x07), 0x0000);

        put_glyph(&mut v, 0x0000, b'#', &[0b1000_0000]);
        put_glyph(&mut v, 0x4000, b'#', &[0b0000_0001]);
        // Cell 0 uses set B (bit3 clear), cell 1 uses set A (bit3 set).
        put_cell(&mut v, 0, b'#', 0x07);
        put_cell(&mut v, 1, b'#', 0x0F);

        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.index_at(0, 0), Some(PALETTE[7]), "set B leftmost dot");
        assert_eq!(frame.index_at(7, 0), Some(PALETTE[0]));
        assert_eq!(frame.index_at(9, 0), Some(PALETTE[0]));
        assert_eq!(frame.index_at(16, 0), Some(PALETTE[15]), "set A eighth dot");

        // Spec: FreeVGA Sequencer Memory Mode `Ext. Mem` — character map
        // selection requires Extended Memory.
        set_seq(
            &mut v,
            VGA_SEQ_MEMORY_MODE,
            VGA_SEQ_MEMORY_MODE_DEFAULT & !0x02,
        );
        assert!(!v.seq_extended_memory());
        assert_eq!(v.text_font_bank_offset(0x0F), 0x0000);
    }

    /// Spec: FreeVGA CRTC Maximum Scan Line — "programmed with the character
    /// height - 1".
    #[test]
    fn maximum_scan_line_sets_the_cell_height_and_frame_height() {
        let mut v = VgaText::new();
        set_crtc(&mut v, VGA_CRTC_MAX_SCAN_LINE, 7);
        assert_eq!(v.text_cell_height(), 8);
        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.height, 200);
        assert_eq!(frame.width, 720);

        set_crtc(&mut v, VGA_CRTC_MAX_SCAN_LINE, 0);
        assert_eq!(v.text_cell_height(), 1);
        assert_eq!(v.render_frame(false).unwrap().height, 25);
    }

    /// Spec: FreeVGA CRTC Start Address (first displayed character) and Offset
    /// ("the address difference between ... two lines of characters"; character
    /// pitch is `Offset * 2`).
    #[test]
    fn start_address_and_offset_place_the_fetched_cells() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        put_glyph(&mut v, 0, b'S', &[0xFF]);
        put_glyph(&mut v, 0, b'R', &[0x80]);

        // Start Address 0x0003 → the top-left cell is counter 3.
        set_crtc(&mut v, VGA_CRTC_START_ADDR_HIGH, 0x00);
        set_crtc(&mut v, VGA_CRTC_START_ADDR_LOW, 0x03);
        assert_eq!(v.text_start_address(), 3);
        put_cell(&mut v, 3, b'S', 0x0F);

        // Offset 0x14 → 40-character row pitch, so row 1 starts at counter 43.
        set_crtc(&mut v, VGA_CRTC_OFFSET, 0x14);
        assert_eq!(v.text_row_pitch_chars(), 40);
        put_cell(&mut v, 43, b'R', 0x0F);

        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(
            frame.index_at(0, 0),
            Some(PALETTE[15]),
            "start address cell"
        );
        assert_eq!(frame.index_at(7, 0), Some(PALETTE[15]));
        // Row 1 begins at scan line 16 with the 'R' glyph's single left dot.
        assert_eq!(frame.index_at(0, 16), Some(PALETTE[15]), "offset row 1");
        assert_eq!(frame.index_at(1, 16), Some(PALETTE[0]));
    }

    /// Spec: FreeVGA CRTC Cursor Location Low — the scan lines between Cursor
    /// Scan Line Start and End "are replaced with the foreground color" in the
    /// cell whose display address matches Cursor Location; Cursor Start bit5
    /// disables the cursor and an End below Start draws nothing.
    #[test]
    fn cursor_registers_replace_scan_lines_with_the_foreground_color() {
        let mut v = VgaText::new();
        // Cursor at cell 1, scan lines 14..=15, attribute foreground 15.
        put_cell(&mut v, 1, b' ', 0x0F);
        set_crtc(&mut v, VGA_CRTC_CURSOR_LOC_HIGH, 0x00);
        set_crtc(&mut v, VGA_CRTC_CURSOR_LOC_LOW, 0x01);
        set_crtc(&mut v, VGA_CRTC_CURSOR_START, 14);
        set_crtc(&mut v, VGA_CRTC_CURSOR_END, 15);
        assert!(!v.crtc_cursor_disabled());

        let frame = v.render_frame(false).expect("text mode renders");
        for scan in 0..16 {
            let want = if (14..=15).contains(&scan) {
                PALETTE[15]
            } else {
                PALETTE[0]
            };
            assert_eq!(frame.index_at(9, scan), Some(want), "cursor scan {scan}");
        }
        // The neighbouring cell has no cursor.
        assert_eq!(frame.index_at(0, 14), Some(PALETTE[0]));

        // Cursor Disable blanks it.
        set_crtc(&mut v, VGA_CRTC_CURSOR_START, VGA_CRTC_CURSOR_DISABLE | 14);
        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.index_at(9, 14), Some(PALETTE[0]));

        // End below Start draws nothing.
        set_crtc(&mut v, VGA_CRTC_CURSOR_START, 15);
        set_crtc(&mut v, VGA_CRTC_CURSOR_END, 14);
        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.index_at(9, 15), Some(PALETTE[0]));
        assert_eq!(frame.index_at(9, 14), Some(PALETTE[0]));
    }

    /// Spec: FreeVGA "VGA Text Mode Operation", Attributes — with Blink Enable
    /// set, attribute bit7 makes the foreground alternate with the background.
    #[test]
    fn blink_attribute_draws_the_foreground_as_background_on_the_off_half() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        assert!(v.atc_blink_enabled());
        put_glyph(&mut v, 0, b'B', &[0xFF]);
        // Attribute 0x87: blink bit set, foreground 7, background bits 6:4 = 0.
        put_cell(&mut v, 0, b'B', 0x87);
        // Attribute 0x07: no blink bit.
        put_cell(&mut v, 1, b'B', 0x07);

        let on = v.render_frame(false).expect("text mode renders");
        assert_eq!(on.index_at(0, 0), Some(PALETTE[7]));
        assert_eq!(on.index_at(9, 0), Some(PALETTE[7]));

        let off = v.render_frame(true).expect("text mode renders");
        assert_eq!(
            off.index_at(0, 0),
            Some(PALETTE[0]),
            "blinked cell is hidden"
        );
        assert_eq!(off.index_at(9, 0), Some(PALETTE[7]), "non-blinking cell");

        // With Blink Enable clear, attribute bit7 is background intensity and
        // the off half changes nothing.
        set_atc(&mut v, VGA_ATC_MODE_CONTROL, VGA_ATC_MODE_LINE_GRAPHICS);
        assert!(!v.atc_blink_enabled());
        let off = v.render_frame(true).expect("text mode renders");
        assert_eq!(off.index_at(0, 0), Some(PALETTE[7]));
        assert_eq!(off.index_at(1, 0), Some(PALETTE[7]));
        // Background is now the full 4-bit field: 0x8 → palette entry 8.
        assert_eq!(off.index_at(0, 1), Some(PALETTE[8]));
    }

    /// The rendered indices pass the whole ATC Internal Palette → Color Select
    /// → PEL Mask chain, not just the palette.
    ///
    /// Spec: FreeVGA Attribute Controller Color Select / Attribute Mode Control
    /// `P54S`; FreeVGA Color Registers PEL Mask.
    #[test]
    fn palette_color_select_and_pel_mask_compose_into_the_frame() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        put_glyph(&mut v, 0, b'C', &[0xFF]);
        put_cell(&mut v, 0, b'C', 0x0E);

        // Color Select bits 3:2 = 11b supply DAC bits 7:6.
        set_atc(&mut v, VGA_ATC_COLOR_SELECT, 0b0000_1100);
        let want = 0xC0 | PALETTE[14];
        assert_eq!(v.text_attr_fg_dac_index(0x0E), want);
        assert_eq!(v.render_frame(false).unwrap().index_at(0, 0), Some(want));

        // PEL Mask is applied last.
        v.port_write(VGA_DAC_PEL_MASK, 1, 0x0F);
        let masked = want & 0x0F;
        assert_eq!(v.text_attr_fg_dac_index(0x0E), masked);
        assert_eq!(v.render_frame(false).unwrap().index_at(0, 0), Some(masked));

        // P54S set replaces palette bits 5:4 with Color Select bits 1:0.
        v.port_write(VGA_DAC_PEL_MASK, 1, u32::from(VGA_DAC_PEL_MASK_DEFAULT));
        set_atc(
            &mut v,
            VGA_ATC_MODE_CONTROL,
            VGA_ATC_MODE_CONTROL_DEFAULT | VGA_ATC_MODE_P54S,
        );
        set_atc(&mut v, VGA_ATC_COLOR_SELECT, 0b0000_0001);
        let want = 0x10 | (PALETTE[14] & 0x0F);
        assert_eq!(v.render_frame(false).unwrap().index_at(0, 0), Some(want));
    }

    /// Spec: IBM PS/2 Video Subsystems Figure 2-79 Mode Control bit2 — with
    /// Line Graphics Enable set, codes `C0h`–`DFh` repeat their eighth dot in
    /// the ninth column; every other code has a background ninth dot.
    #[test]
    fn ninth_dot_repeats_only_for_line_graphics_codes_under_lge() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        assert!(v.atc_line_graphics_enabled());
        put_glyph(&mut v, 0, 0xC4, &[0xFF]);
        put_glyph(&mut v, 0, 0xBF, &[0xFF]);
        put_glyph(&mut v, 0, 0xE0, &[0xFF]);
        put_cell(&mut v, 0, 0xC4, 0x0F);
        put_cell(&mut v, 1, 0xBF, 0x0F);
        put_cell(&mut v, 2, 0xE0, 0x0F);

        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.index_at(8, 0), Some(PALETTE[15]), "C4h repeats");
        assert_eq!(frame.index_at(17, 0), Some(PALETTE[0]), "BFh does not");
        assert_eq!(frame.index_at(26, 0), Some(PALETTE[0]), "E0h does not");

        // The eighth dot must be clear for the ninth to follow it.
        put_glyph(&mut v, 0, 0xC4, &[0xFE]);
        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.index_at(7, 0), Some(PALETTE[0]));
        assert_eq!(frame.index_at(8, 0), Some(PALETTE[0]));

        // Clearing Line Graphics Enable blanks the ninth dot for C0h-DFh too.
        put_glyph(&mut v, 0, 0xC4, &[0xFF]);
        set_atc(&mut v, VGA_ATC_MODE_CONTROL, VGA_ATC_MODE_BLINK);
        assert!(!v.atc_line_graphics_enabled());
        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.index_at(7, 0), Some(PALETTE[15]));
        assert_eq!(frame.index_at(8, 0), Some(PALETTE[0]));
    }

    /// Spec: FreeVGA Clocking Mode bit0 set selects 8 dots per character, so
    /// there is no ninth column at all.
    #[test]
    fn eight_dot_mode_removes_the_ninth_column() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        put_glyph(&mut v, 0, 0xC4, &[0xFF]);
        put_cell(&mut v, 0, 0xC4, 0x0F);
        put_cell(&mut v, 1, 0xC4, 0x0F);

        set_seq(&mut v, VGA_SEQ_CLOCKING_MODE, VGA_SEQ_CLOCKING_8DOT);
        assert_eq!(v.text_cell_width(), 8);
        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.width, 640);
        // Cell 1 starts at dot 8, so there is no background gap.
        assert_eq!(frame.index_at(7, 0), Some(PALETTE[15]));
        assert_eq!(frame.index_at(8, 0), Some(PALETTE[15]));
    }

    /// Spec: FreeVGA "VGA Text Mode Operation", Attributes — underline needs
    /// attribute bits 2:0 = `001b` and bits 6:4 = `000b`, and the mode-03h
    /// Underline Location default puts the line outside a 16-line cell.
    #[test]
    fn underline_attribute_replaces_the_underline_location_row() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        assert!(v.text_attr_underlines(0x01));
        assert!(v.text_attr_underlines(0x81));
        assert!(!v.text_attr_underlines(0x11));
        assert!(!v.text_attr_underlines(0x07));

        put_cell(&mut v, 0, b' ', 0x01);
        // Default 0x1F is past a 16-line cell: nothing is drawn.
        assert_eq!(v.crtc_underline_scanline(), 0x1F);
        let frame = v.render_frame(false).expect("text mode renders");
        assert!((0..16).all(|y| frame.index_at(0, y) == Some(PALETTE[0])));

        set_crtc(&mut v, VGA_CRTC_UNDERLINE_LOCATION, 15);
        let frame = v.render_frame(false).expect("text mode renders");
        for dot in 0..9 {
            assert_eq!(frame.index_at(dot, 15), Some(PALETTE[1]), "dot {dot}");
        }
        assert_eq!(frame.index_at(0, 14), Some(PALETTE[0]));
    }

    /// Graphics programming is reported rather than rendered as text.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-74 Miscellaneous bit0
    /// (Graphics/Alphanumeric) and FreeVGA Attribute Mode Control `ATGE`.
    #[test]
    fn graphics_programming_reports_unsupported_and_renders_nothing() {
        let mut v = VgaText::new();
        set_gc(
            &mut v,
            VGA_GC_MISC,
            VGA_GC_MISC_DEFAULT | VGA_GC_MISC_GRAPHICS_MODE,
        );
        assert_eq!(v.render_mode(), VgaRenderMode::Unsupported);
        assert!(v.render_frame(false).is_none());

        let mut v = VgaText::new();
        set_atc(
            &mut v,
            VGA_ATC_MODE_CONTROL,
            VGA_ATC_MODE_CONTROL_DEFAULT | VGA_ATC_MODE_ATGE,
        );
        assert_eq!(v.render_mode(), VgaRenderMode::Unsupported);
        assert!(v.render_frame(false).is_none());
    }

    /// The host conversion scales 6-bit DAC components to 8 bits by bit
    /// replication and emits opaque RGBA.
    #[test]
    fn frame_rgba8_expands_dac_entries_to_opaque_rgba() {
        let mut v = VgaText::new();
        disable_cursor(&mut v);
        v.dac_ram[usize::from(PALETTE[14])] = [0x3F, 0x00, 0x15];
        put_glyph(&mut v, 0, b'D', &[0b1000_0000]);
        put_cell(&mut v, 0, b'D', 0x0E);

        let frame = v.render_frame(false).expect("text mode renders");
        assert_eq!(frame.index_at(0, 0), Some(PALETTE[14]));
        assert_eq!(v.dac_rgb6(PALETTE[14]), [0x3F, 0x00, 0x15]);

        let rgba = v.frame_rgba8(&frame);
        assert_eq!(rgba.len(), frame.pixels.len() * 4);
        assert_eq!(&rgba[..4], &[0xFF, 0x00, 0x55, 0xFF]);
        // Background index 0 is black in the reset DAC RAM.
        assert_eq!(&rgba[4..8], &[0x00, 0x00, 0x00, 0xFF]);
    }
}

/// One display memory (M2 round 3, slice 2).
///
/// Round 2 kept two backing stores: a 32 KiB interleaved text buffer for
/// `0xB8000` and four maps for everything else. Real hardware has one memory.
/// These tests pin the property that made retiring the split safe — under the
/// mode-03h programming the Graphics Controller resolves a text access to the
/// same map and offset the alphanumeric view uses — and the places where the
/// unified model genuinely differs.
#[cfg(test)]
mod unified_display_memory_tests {
    use super::*;

    fn set_gc(v: &mut VgaText, index: u8, value: u8) {
        v.port_write(VGA_GC_INDEX, 1, u32::from(index));
        v.port_write(VGA_GC_DATA, 1, u32::from(value));
    }

    fn set_seq(v: &mut VgaText, index: u8, value: u8) {
        v.port_write(VGA_SEQ_INDEX, 1, u32::from(index));
        v.port_write(VGA_SEQ_DATA, 1, u32::from(value));
    }

    /// A guest text write reaches the maps the character generator fetches.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figure 2-33 (odd/even sends even
    /// addresses to maps 0+2 and odd to maps 1+3) + Figure 2-29 (Map Mask
    /// `0x03` narrows that to maps 0 and 1) + FreeVGA "VGA Text Mode
    /// Operation" (map 0 characters, map 1 attributes).
    #[test]
    fn a_guest_text_write_lands_in_the_maps_the_renderer_reads() {
        let mut v = VgaText::new();
        assert!(v.mmio_write_u8(VGA_TEXT_BASE, b'H'));
        assert!(v.mmio_write_u8(VGA_TEXT_BASE + 1, 0x1E));

        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 0), Some(b'H'));
        assert_eq!(v.plane_byte(VGA_TEXT_ATTR_PLANE, 0), Some(0x1E));
        // Map Mask 0x03 keeps maps 2 and 3 out, so the font bank is untouched.
        assert_eq!(v.plane_byte(VGA_FONT_PLANE, 0), Some(0x00));
        assert_eq!(v.plane_byte(3, 0), Some(0x00));

        assert_eq!(v.char_at(0, 0), Some(b'H'));
        assert_eq!(v.attr_at(0, 0), Some(0x1E));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'H'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE + 1), Some(0x1E));
        assert_eq!(v.mmio_read_u8(VGA_TEXT_BASE), Some(b'H'));
        assert_eq!(v.mmio_read_u8(VGA_TEXT_BASE + 1), Some(0x1E));
    }

    /// There is no second store to fall out of sync: a direct map write shows
    /// up in every text view, and a text write shows up in the maps.
    #[test]
    fn map_writes_and_text_writes_reach_the_same_bytes() {
        let mut v = VgaText::new();
        assert!(v.set_plane_byte(VGA_TEXT_CHAR_PLANE, 0, b'Z'));
        assert!(v.set_plane_byte(VGA_TEXT_ATTR_PLANE, 0, 0x4E));
        assert_eq!(v.char_at(0, 0), Some(b'Z'));
        assert_eq!(v.attr_at(0, 0), Some(0x4E));
        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(b'Z'));
        assert_eq!(v.read_u8(VGA_TEXT_BASE + 1), Some(0x4E));

        assert!(v.put_char(0, 0, b'Q', 0x1F));
        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 0), Some(b'Q'));
        assert_eq!(v.plane_byte(VGA_TEXT_ATTR_PLANE, 0), Some(0x1F));
    }

    /// A graphics-window write at the same display offset is visible to the
    /// text helpers — the behavior the split backing stores used to hide.
    ///
    /// Spec: IBM Figure 2-75 — the `0xA0000` 64 KB window decodes offsets from
    /// its own base, so window offset 0 is the same display memory the
    /// alphanumeric cell at CRTC counter 0 uses.
    #[test]
    fn a_graphics_window_write_is_visible_to_the_text_helpers() {
        let mut v = VgaText::new();
        set_gc(
            &mut v,
            VGA_GC_MISC,
            VGA_GC_MISC_GRAPHICS_MODE
                | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
        );
        set_seq(
            &mut v,
            VGA_SEQ_MEMORY_MODE,
            VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
        );
        set_seq(&mut v, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES);

        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE, 0x41));
        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 0), Some(0x41));
        assert_eq!(v.plane_byte(VGA_TEXT_ATTR_PLANE, 0), Some(0x41));
        assert_eq!(v.char_at(0, 0), Some(0x41));
        assert_eq!(v.attr_at(0, 0), Some(0x41));
    }

    /// Every guest access now runs the Graphics Controller, so a text read
    /// loads all four latches. The host helpers still do not.
    ///
    /// Spec: OSDev VGA Hardware "The Latches" — a system read loads all four.
    #[test]
    fn a_guest_text_read_loads_the_latches_but_a_host_read_does_not() {
        let mut v = VgaText::new();
        assert!(v.set_plane_byte(0, 0, 0x11));
        assert!(v.set_plane_byte(1, 0, 0x22));
        assert!(v.set_plane_byte(2, 0, 0x33));
        assert!(v.set_plane_byte(3, 0, 0x44));

        assert_eq!(v.read_u8(VGA_TEXT_BASE), Some(0x11));
        assert_eq!(v.gc_latches, [0; VGA_PLANE_COUNT]);

        assert_eq!(v.mmio_read_u8(VGA_TEXT_BASE), Some(0x11));
        assert_eq!(v.gc_latches, [0x11, 0x22, 0x33, 0x44]);
    }

    /// A guest text write now goes through the write path, so Map Mask, write
    /// modes and the Bit Mask apply to `0xB8000` — they could not before.
    ///
    /// Spec: IBM Figure 2-73 Write Mode Definitions; Figure 2-29 Map Mask;
    /// Figure 2-77 Bit Mask.
    #[test]
    fn write_modes_and_map_mask_now_apply_at_b8000() {
        let mut v = VgaText::new();
        // Map Mask 0x02 keeps the character map out of an even-address write.
        set_seq(&mut v, VGA_SEQ_MAP_MASK, 0b0010);
        assert!(v.mmio_write_u8(VGA_TEXT_BASE, b'X'));
        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 0), Some(b' '));

        // Bit Mask on a normal text write.
        set_seq(&mut v, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_DEFAULT);
        set_gc(&mut v, VGA_GC_BIT_MASK, 0x0F);
        v.mmio_read_u8(VGA_TEXT_BASE); // load latches with the space
        assert!(v.mmio_write_u8(VGA_TEXT_BASE, 0xFF));
        assert_eq!(
            v.plane_byte(VGA_TEXT_CHAR_PLANE, 0),
            Some((b' ' & 0xF0) | 0x0F)
        );
    }

    /// Reset restores the 80×25 blank screen into display memory itself.
    #[test]
    fn reset_puts_the_blank_screen_into_display_memory() {
        let mut v = VgaText::new();
        assert!(v.set_plane_byte(VGA_TEXT_CHAR_PLANE, 0, 0xAA));
        assert!(v.set_plane_byte(VGA_FONT_PLANE, 0x1234, 0xBB));
        v.reset();

        assert_eq!(v.plane_byte(VGA_TEXT_CHAR_PLANE, 0), Some(VGA_DEFAULT_CHAR));
        assert_eq!(v.plane_byte(VGA_TEXT_ATTR_PLANE, 0), Some(VGA_DEFAULT_ATTR));
        let last = (VGA_TEXT_COLS * VGA_TEXT_ROWS - 1) * VGA_CELL_BYTES;
        assert_eq!(
            v.plane_byte(VGA_TEXT_CHAR_PLANE, last),
            Some(VGA_DEFAULT_CHAR)
        );
        assert_eq!(v.plane_byte(VGA_FONT_PLANE, 0x1234), Some(0x00));
        assert_eq!(v.char_at(24, 79), Some(VGA_DEFAULT_CHAR));
    }
}

/// Chain-4 256-color graphics display fetch — the mode-13h path
/// (M2 round 3, slice 3).
///
/// This is the *only* graphics fetch this model has. Planar 16-color modes
/// (0Dh / 0Eh / 10h / 12h) have no renderer, and there is no VBE.
#[cfg(test)]
mod graphics256_tests {
    use super::*;

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

    /// The register values IBM mode 13h programs, for the fields this device
    /// models: `0xA0000` 64 KB graphics window, 256-color shift, chain-4 with
    /// all maps writable, 8-bit attribute straight to the DAC, doubleword
    /// addressing with an Offset of `0x28` (a 320-byte row stride).
    fn program_mode13h(v: &mut VgaText) {
        // A BIOS mode set clears display memory; clear it here too so the
        // reset 80×25 blank-screen fill (which now lives in maps 0 and 1)
        // does not show up as pixels.
        v.planes.fill(0);
        set_gc(
            v,
            VGA_GC_MISC,
            VGA_GC_MISC_GRAPHICS_MODE
                | (VGA_GC_MEMORY_MAP_A0000_64K << VGA_GC_MISC_MEMORY_MAP_SHIFT),
        );
        set_gc(v, VGA_GC_MODE, VGA_GC_MODE_SHIFT256);
        set_seq(
            v,
            VGA_SEQ_MEMORY_MODE,
            VGA_SEQ_MEMORY_MODE_EXTENDED
                | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE
                | VGA_SEQ_MEMORY_MODE_CHAIN4,
        );
        set_seq(v, VGA_SEQ_MAP_MASK, VGA_SEQ_MAP_MASK_PLANES);
        set_atc(
            v,
            VGA_ATC_MODE_CONTROL,
            VGA_ATC_MODE_ATGE | VGA_ATC_MODE_8BIT,
        );
        set_crtc(v, VGA_CRTC_UNDERLINE_LOCATION, VGA_CRTC_UNDERLINE_DW);
        set_crtc(v, VGA_CRTC_OFFSET, VGA_CRTC_OFFSET_DEFAULT);
    }

    /// Spec: IBM mode 13h — 320×200×256. The whole signature must be present.
    #[test]
    fn the_mode13h_signature_selects_the_chain4_256_color_fetch() {
        let mut v = VgaText::new();
        assert_eq!(v.render_mode(), VgaRenderMode::Text);

        program_mode13h(&mut v);
        assert!(v.is_mode13h_programming());
        assert_eq!(v.render_mode(), VgaRenderMode::Graphics256Chain4);
        assert_eq!(v.crtc_address_multiplier(), 4);
        assert_eq!(v.graphics_row_stride_bytes(), 320);

        let frame = v.render_frame(false).expect("mode 13h renders");
        assert_eq!(frame.width, VGA_MODE13_WIDTH);
        assert_eq!(frame.height, VGA_MODE13_HEIGHT);
        assert_eq!(frame.pixels.len(), 320 * 200);
        assert_eq!(frame.mode, VgaRenderMode::Graphics256Chain4);
    }

    /// Any missing piece of the signature is reported rather than rendered.
    #[test]
    fn partial_256_color_programming_is_unsupported() {
        for drop in 0..5 {
            let mut v = VgaText::new();
            program_mode13h(&mut v);
            match drop {
                0 => set_gc(&mut v, VGA_GC_MISC, VGA_GC_MISC_DEFAULT),
                1 => set_gc(&mut v, VGA_GC_MODE, 0x00),
                2 => set_seq(
                    &mut v,
                    VGA_SEQ_MEMORY_MODE,
                    VGA_SEQ_MEMORY_MODE_EXTENDED | VGA_SEQ_MEMORY_MODE_ODD_EVEN_DISABLE,
                ),
                3 => set_atc(&mut v, VGA_ATC_MODE_CONTROL, VGA_ATC_MODE_ATGE),
                _ => set_atc(&mut v, VGA_ATC_MODE_CONTROL, VGA_ATC_MODE_8BIT),
            }
            assert!(!v.is_mode13h_programming(), "case {drop}");
            // Dropping ATGE alone still leaves Graphics/Alphanumeric set, so
            // this is graphics programming with no renderer, not text.
            assert_eq!(v.render_mode(), VgaRenderMode::Unsupported, "case {drop}");
            assert!(v.render_frame(false).is_none(), "case {drop}");
        }
    }

    /// A guest byte written to `0xA0000 + n` is the pixel displayed at *n*.
    ///
    /// Spec: IBM Figure 2-34 — chain 4 selects the map from A1:A0, so
    /// consecutive display bytes rotate through maps 0, 1, 2, 3 at one shared
    /// per-map offset.
    #[test]
    fn consecutive_pixels_rotate_through_the_four_maps() {
        let mut v = VgaText::new();
        program_mode13h(&mut v);
        for n in 0..8u8 {
            assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE + u64::from(n), 0x10 + n));
        }

        // Bytes 0-3 share per-map offset 0; bytes 4-7 share offset 4.
        assert_eq!(v.plane_byte(0, 0), Some(0x10));
        assert_eq!(v.plane_byte(1, 0), Some(0x11));
        assert_eq!(v.plane_byte(2, 0), Some(0x12));
        assert_eq!(v.plane_byte(3, 0), Some(0x13));
        assert_eq!(v.plane_byte(0, 4), Some(0x14));
        assert_eq!(v.plane_byte(3, 4), Some(0x17));

        let frame = v.render_frame(false).expect("mode 13h renders");
        for n in 0..8usize {
            assert_eq!(frame.index_at(n, 0), Some(0x10 + n as u8), "pixel {n}");
        }
        // Nothing else was written, so the rest of the row is index 0.
        assert_eq!(frame.index_at(8, 0), Some(0x00));
    }

    /// Row *r* starts one 320-byte stride further into display memory.
    ///
    /// Spec: FreeVGA CRTC Offset Register — the row stride is
    /// `Offset * 2 * MemoryAddressSize`, and mode 13h uses doubleword
    /// addressing, giving `0x28 * 2 * 4` = 320.
    #[test]
    fn rows_are_one_offset_stride_apart() {
        let mut v = VgaText::new();
        program_mode13h(&mut v);
        // First pixel of row 1 is display byte 320.
        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE + 320, 0x5A));
        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE + 320 + 319, 0x6B));

        let frame = v.render_frame(false).expect("mode 13h renders");
        assert_eq!(frame.index_at(0, 1), Some(0x5A));
        assert_eq!(frame.index_at(319, 1), Some(0x6B));
        assert_eq!(frame.index_at(0, 0), Some(0x00));
        assert_eq!(frame.index_at(0, 2), Some(0x00));
    }

    /// Start Address moves the whole picture, scaled by the addressing
    /// multiplier. Spec: FreeVGA CRTC Start Address Low.
    #[test]
    fn start_address_moves_the_graphics_origin() {
        let mut v = VgaText::new();
        program_mode13h(&mut v);
        // Counter 2 with doubleword addressing is display byte 8.
        set_crtc(&mut v, VGA_CRTC_START_ADDR_HIGH, 0x00);
        set_crtc(&mut v, VGA_CRTC_START_ADDR_LOW, 0x02);
        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE + 8, 0x77));
        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE + 9, 0x88));

        let frame = v.render_frame(false).expect("mode 13h renders");
        assert_eq!(frame.index_at(0, 0), Some(0x77));
        assert_eq!(frame.index_at(1, 0), Some(0x88));
    }

    /// The 8-bit pixel value goes to the DAC directly: the Internal Palette
    /// and Color Select take no part, and only the PEL Mask is applied.
    ///
    /// Spec: FreeVGA Attribute Mode Control `8BIT` + Color Select ("In mode 13
    /// hex, the 8-bit attribute is the digital color value to the video DAC").
    #[test]
    fn the_internal_palette_is_bypassed_and_only_the_pel_mask_applies() {
        let mut v = VgaText::new();
        program_mode13h(&mut v);
        // Repaint palette entry 5 and set Color Select; neither may show up.
        set_atc(&mut v, 0x05, 0x2A);
        set_atc(&mut v, VGA_ATC_COLOR_SELECT, 0b0000_1111);
        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE, 0x05));
        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE + 1, 0xC5));

        let frame = v.render_frame(false).expect("mode 13h renders");
        assert_eq!(frame.index_at(0, 0), Some(0x05));
        assert_eq!(frame.index_at(1, 0), Some(0xC5));

        v.port_write(VGA_DAC_PEL_MASK, 1, 0x3F);
        let frame = v.render_frame(false).expect("mode 13h renders");
        assert_eq!(frame.index_at(0, 0), Some(0x05));
        assert_eq!(frame.index_at(1, 0), Some(0xC5 & 0x3F));
    }

    /// The RGBA expansion works the same for a graphics frame.
    #[test]
    fn graphics_frames_expand_through_the_dac_like_text_frames() {
        let mut v = VgaText::new();
        program_mode13h(&mut v);
        v.dac_ram[0x21] = [0x00, 0x3F, 0x15];
        assert!(v.mmio_write_u8(VGA_WINDOW_A0000_BASE, 0x21));

        let frame = v.render_frame(false).expect("mode 13h renders");
        let rgba = v.frame_rgba8(&frame);
        assert_eq!(rgba.len(), 320 * 200 * 4);
        assert_eq!(&rgba[..4], &[0x00, 0xFF, 0x55, 0xFF]);
    }
}
