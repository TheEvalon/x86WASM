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
//! - 32 KiB text plane buffer at `VGA_TEXT_BASE`…`VGA_TEXT_END`
//! - Byte R/W; reset fills first 80×25 with space + attribute `0x07`
//! - Helpers for tests (`char_at` / `attr_at` / `put_char`)
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
//!   read mode 0 (Read Map Select, or A1:A0 in Chain 4) or read mode 1 (Color
//!   Compare / Color Don't Care); [`VgaText::gc_write_u8`] applies write modes
//!   0–3 with Set/Reset + Enable Set/Reset, Data Rotate + Function Select,
//!   Bit Mask, and Map Mask plane write enables. This path is host-callable
//!   only — `MachineBus` CPU MMIO still uses the legacy text buffer.
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
//! - The Graphics Controller data path is not wired to CPU MMIO: `read_u8` /
//!   `write_u8` (used by `MachineBus`) still address the interleaved text
//!   buffer directly, so guest writes do not flow through write modes, latches,
//!   Map Mask, or the plane decode
//! - Graphics Mode bit4 host odd/even *read* addressing does not steer read
//!   mode 0 map selection (IBM Figure 2-71's odd/even note is ambiguous);
//!   Shift Register Interleave and 256-Color Shift Mode have no effect
//! - No display fetch from [`VgaText::planes`]: character generation, planar
//!   pixel output, and Chain-4/doubleword display addressing are absent
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
const _: () = assert!(
    VGA_GC_MODE_DEFAULT & VGA_GC_MODE_WRITE_MASK == 0
        && VGA_GC_MODE_DEFAULT & VGA_GC_MODE_READ == 0
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

/// Color text-mode frame buffer + CRTC + Sequencer + GC + ATC + Misc + DAC stubs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VgaText {
    /// Raw plane bytes (char/attr interleaved).
    pub mem: Vec<u8>,
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
    /// Display memory maps: [`VGA_PLANE_COUNT`] × [`VGA_PLANE_SIZE`] bytes,
    /// map-major (map `p` offset `o` at `p * VGA_PLANE_SIZE + o`).
    ///
    /// Reached through [`VgaText::gc_read_u8`] / [`VgaText::gc_write_u8`];
    /// the legacy interleaved text buffer [`VgaText::mem`] stays separate.
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
            mem: vec![0; VGA_TEXT_SIZE],
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
    pub fn reset(&mut self) {
        self.mem.fill(0);
        let cells = VGA_TEXT_COLS * VGA_TEXT_ROWS;
        for i in 0..cells {
            let off = i * VGA_CELL_BYTES;
            self.mem[off] = VGA_DEFAULT_CHAR;
            self.mem[off + 1] = VGA_DEFAULT_ATTR;
        }
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

    /// CPU display window claimed by the video subsystem (`base..end`).
    ///
    /// This slice keeps the historical color text window; Graphics Controller
    /// Miscellaneous Memory Map Select decode is a separate slice.
    pub fn display_window(&self) -> (u64, u64) {
        (VGA_TEXT_BASE, VGA_TEXT_END)
    }

    /// Addressing model currently programmed in Sequencer Memory Mode.
    ///
    /// Spec: IBM PS/2 Video Subsystems Figures 2-33 / 2-34 — Chain 4 takes
    /// precedence over odd/even (it replaces map selection entirely with
    /// A1/A0); otherwise OE = 0 gives odd/even and OE = 1 gives planar
    /// Map-Mask-only addressing.
    pub fn plane_addressing(&self) -> VgaPlaneAddressing {
        if self.seq_chain4_enabled() {
            VgaPlaneAddressing::Chain4
        } else if self.seq_odd_even_enabled() {
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
                // through a mask of rotated data ANDed with the Bit Mask.
                _ => {
                    let mask = rotated & bit_mask;
                    Self::blend_with_latch(Self::expand_map_bit(set_reset, plane), latch, mask)
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

    pub fn read_u8(&self, addr: u64) -> Option<u8> {
        if !Self::owns_addr(addr) || !self.misc_ram_enable() {
            // Choice when RAM Enable clear: same `None` as out-of-window so
            // `MachineBus` falls through to open-bus / PhysMem (does not expose
            // plane data). Spec: FreeVGA / IBM Misc Output bit1.
            return None;
        }
        let off = (addr - VGA_TEXT_BASE) as usize;
        Some(self.mem[off])
    }

    pub fn write_u8(&mut self, addr: u64, val: u8) -> bool {
        if !Self::owns_addr(addr) || !self.misc_ram_enable() {
            // Choice when RAM Enable clear: ignore write (`false`), plane
            // unchanged — same "not handled" as out-of-window.
            return false;
        }
        let off = (addr - VGA_TEXT_BASE) as usize;
        self.mem[off] = val;
        true
    }

    /// Byte offset of a visible text cell relative to CRTC Start Address.
    ///
    /// Spec: FreeVGA CRT Controller — Start Address is the character index of
    /// the first displayed cell; Offset is the logical line width in words.
    /// Host viewport helpers index `(row, col)` as
    /// `start + row*pitch + col` where `pitch = Offset * 2` character cells
    /// ([`VgaText::text_row_pitch_chars`]). When that index exceeds the 32 KiB
    /// plane, wrap within the plane (FreeVGA notes display wrap in video
    /// memory). CPU MMIO (`read_u8`/`write_u8`) stays absolute at `0xB8000`.
    fn cell_offset(&self, row: usize, col: usize) -> Option<usize> {
        if row >= VGA_TEXT_ROWS || col >= VGA_TEXT_COLS {
            return None;
        }
        let chars_in_plane = VGA_TEXT_SIZE / VGA_CELL_BYTES;
        let cell =
            (usize::from(self.text_start_address()) + row * self.text_row_pitch_chars() + col)
                % chars_in_plane;
        Some(cell * VGA_CELL_BYTES)
    }

    pub fn char_at(&self, row: usize, col: usize) -> Option<u8> {
        let off = self.cell_offset(row, col)?;
        Some(self.mem[off])
    }

    pub fn attr_at(&self, row: usize, col: usize) -> Option<u8> {
        let off = self.cell_offset(row, col)?;
        Some(self.mem[off + 1])
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: u8, attr: u8) -> bool {
        let Some(off) = self.cell_offset(row, col) else {
            return false;
        };
        self.mem[off] = ch;
        self.mem[off + 1] = attr;
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
        // Spec: IBM VGA text — default light-gray-on-black cells.
        let v = VgaText::new();
        assert_eq!(v.mem.len(), VGA_TEXT_SIZE);
        assert_eq!(v.char_at(0, 0), Some(b' '));
        assert_eq!(v.attr_at(0, 0), Some(0x07));
        assert_eq!(v.char_at(24, 79), Some(b' '));
        assert_eq!(v.attr_at(24, 79), Some(0x07));
        // Beyond 80×25 within the 32 KiB plane remains 0 after reset.
        assert_eq!(v.mem[80 * 25 * 2], 0);
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

    /// The addressing model is register state only: the legacy text-plane CPU
    /// path at `0xB8000` keeps working while chain-4 is programmed.
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

        set_gc_reg(&mut v, VGA_GC_READ_MAP_SELECT, 1);
        assert_eq!(v.gc_read_u8(VGA_TEXT_BASE), Some(0x1F));
        assert_eq!(v.gc_latches, [b'Z', 0x1F, 0, 0]);
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
