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
//!   character address into the refresh buffer). Maximum Scan Line `0x09`
//!   (bits 4:0 = character cell height − 1; bit5 Start Vertical Blanking bit9;
//!   bit6 Line Compare bit9; bit7 Scan Doubling; mode-03h reset default `0x0F`
//!   for 16 scanlines). Offset `0x13` (logical line width in words; mode-03h
//!   reset default `0x28` for 80-column text). Underline Location `0x14`
//!   (bits 4:0 underline scanline − 1; bit5 DIV4; bit6 DW; mode-03h reset
//!   default `0x1F`). Vertical Retrace End `0x11` bit7 Protect: when set,
//!   writes to indexes `0x00`–`0x07` are ignored except Overflow (`0x07`) bit4
//!   (Line Compare bit8); indexes `>= 0x08` (including Maximum Scan Line,
//!   Offset, and Underline Location) remain writable.
//! - OSDev VGA Hardware / FreeVGA Sequencer Registers — Address `0x3C4`, Data
//!   `0x3C5`; indexes `0x00`–`0x04` (Reset, Clocking Mode, Map Mask, Character
//!   Map Select, Memory Mode).
//! - OSDev VGA Hardware / FreeVGA Graphics Registers — Address `0x3CE`, Data
//!   `0x3CF`; indexes `0x00`–`0x08` (Set/Reset, Enable Set/Reset, Color Compare,
//!   Data Rotate, Read Map Select, Graphics Mode, Miscellaneous, Color Don't
//!   Care, Bit Mask).
//! - OSDev VGA Hardware / FreeVGA Attribute Controller Registers — Address/Data
//!   at `0x3C0` (flip-flop), Data Read at `0x3C1`; indexes `0x00`–`0x14`
//!   (palette `0x00`–`0x0F`, Mode Control `0x10`, Overscan `0x11`, Color Plane
//!   Enable `0x12`, Horizontal PEL Panning `0x13`, Color Select `0x14`). Reading
//!   Input Status #1 (color `0x3DA` / mono `0x3BA`) resets the flip-flop to
//!   address state.
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
//!   text-mode cursor character offset / row-col; Maximum Scan Line `0x09`
//!   store/readback with mode-03h reset default `0x0F` (Protect does not block);
//!   Vertical Retrace End `0x11` bit7 Protect blocks writes to indexes
//!   `0x00`–`0x07` (Overflow bit4 still writable; no host cursor glyph render,
//!   max-scan glyph height, or CRTC timing)
//! - Sequencer index/data noop: latch index on `0x3C4`, store/read register file
//!   on `0x3C5` with mode-03h-class reset defaults (no timing/plane side effects)
//! - Graphics Controller index/data noop: latch index on `0x3CE`, store/read
//!   register file on `0x3CF` with mode-03h-class reset defaults (no write-mode /
//!   map / bitmask side effects)
//! - Attribute Controller noop: address/data flip-flop on `0x3C0`, data read on
//!   `0x3C1`, flip-flop reset via Input Status #1 (active IOAS map) (no palette /
//!   mode-control / render side effects)
//! - Input Status #1: ATC flip-flop reset + deterministic display-enable /
//!   vertical-retrace status bits (read-phase counter); port selected by Misc
//!   Output IOAS (`0x3DA` color / `0x3BA` mono)
//! - Misc Output store/readback (`0x3C2`/`0x3CC`); IOAS bit remaps Input Status
//!   #1 and CRTC index/data ownership; RAM Enable (bit1) gates CPU text-plane
//!   `read_u8`/`write_u8` (not clock select)
//! - DAC / PEL store/readback: write index `0x3C8`, data `0x3C9` (R→G→B), read
//!   index write / state read `0x3C7`; 256×3 RAM with mode-03h-ish defaults
//! - PEL Mask `0x3C6` R/W store/readback (default `0xFF`); display-path AND is
//!   not applied yet (no host render); does **not** alter `0x3C9` palette
//!   programming (FreeVGA/RBIL/Abrash document mask on pixel-index lookup only)
//!
//! # Unsupported (explicit)
//!
//! - ATC / Sequencer / GC timing, ATC→DAC remap side effects, blink, PEL pan,
//!   plane-enable, map-mask, write-mode, read-map, or bitmask side effects on
//!   the text plane
//! - PEL Mask display-path application (pixel index AND before DAC lookup) —
//!   deferred until host render; hidden-DAC unlock via repeated `0x3C6` reads
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
/// CRTC Overflow Register index (holds Line Compare bit8 in bit4).
///
/// Spec: FreeVGA CRT Controller Registers / IBM VGA — index `0x07`.
pub const VGA_CRTC_OVERFLOW: u8 = 0x07;
/// Overflow bit4 — Line Compare bit8; remains writable under Protect.
///
/// Spec: FreeVGA Vertical Retrace End Protect — indexes `0x00`–`0x07` ignore
/// writes when Protect is set, except this Overflow bit.
pub const VGA_CRTC_OVERFLOW_LINE_COMPARE_BIT8: u8 = 0x10;
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
/// (`>= 0x08`). Pitch/render side effects are out of scope (store/readback
/// only).
pub const VGA_CRTC_OFFSET: u8 = 0x13;
/// Mode-03h-class Offset reset default (`0x28` = 40 words → 80 columns).
///
/// Spec: FreeVGA / IBM VGA alphanumeric mode 03h — Offset `0x28` for 80-column
/// text (next character row starts 40 words after the previous). Store/readback
/// only; no pitch side effects in host render.
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
/// Map Mask `0x03` (planes 0+1), Character Map Select `0x00`,
/// Memory Mode `0x02` (extended memory enable; odd/even + chain-4 clear).
pub const VGA_SEQ_DEFAULTS: [u8; VGA_SEQ_REG_COUNT] = [0x03, 0x00, 0x03, 0x00, 0x02];
/// Sequencer index: Clocking Mode register. Spec: FreeVGA Sequencer Registers.
pub const VGA_SEQ_CLOCKING_MODE: u8 = 0x01;
/// Clocking Mode bit0 — 8/9 Dot Mode (1 = 8 dots/char, 0 = 9). Spec: FreeVGA.
pub const VGA_SEQ_CLOCKING_8DOT: u8 = 0x01;
/// Default Clocking Mode has 9-dot characters (bit0 clear).
const _: () =
    assert!((VGA_SEQ_DEFAULTS[VGA_SEQ_CLOCKING_MODE as usize] & VGA_SEQ_CLOCKING_8DOT) == 0);

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

/// Mode-03h-class Graphics Controller reset defaults (store/readback only).
///
/// Spec: FreeVGA Graphics Registers / OSDev VGA Hardware / IBM VGA mode-03h —
/// SeaBIOS-class text programming: Set/Reset `0x00`, Enable Set/Reset `0x00`,
/// Color Compare `0x00`, Data Rotate `0x00`, Read Map Select `0x00`,
/// Graphics Mode `0x10` (host odd/even), Miscellaneous `0x0E` (odd/even +
/// memory map `B8000`), Color Don't Care `0x00`, Bit Mask `0xFF`.
pub const VGA_GC_DEFAULTS: [u8; VGA_GC_REG_COUNT] =
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF];

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

/// Mode-03h-class Attribute Controller reset defaults (store/readback only).
///
/// Spec: FreeVGA / IBM VGA / Abrash mode-set palette — internal palette
/// `00/01/02/03/04/05/14/07/38/39/3A/3B/3C/3D/3E/3F`; Mode Control `0x0C`
/// (BLINK|LGE, alphanumeric); Overscan `0x00`; Color Plane Enable `0x0F`;
/// Horizontal PEL Panning `0x08`; Color Select `0x00`.
pub const VGA_ATC_DEFAULTS: [u8; VGA_ATC_REG_COUNT] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
    0x0C, 0x00, 0x0F, 0x08, 0x00,
];

/// DAC / PEL Mask Register (R/W).
///
/// Spec: FreeVGA Color Registers / OSDev VGA Hardware / IBM VGA / RBIL —
/// ANDed with the color index of each displayed pixel before DAC lookup.
/// Default `0xFF` (no masking). Store/readback only in this stub; the display
/// AND is deferred until host render. Does not affect [`VGA_DAC_DATA`] palette
/// programming (datasheets describe display-path lookup only).
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
/// readback only — no ATC→DAC remap or host render.
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
    /// PEL Mask (`0x3C6`): display-path color-index AND (store/readback; default
    /// [`VGA_DAC_PEL_MASK_DEFAULT`]). Not applied to [`VGA_DAC_DATA`] R/W.
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
    /// except Maximum Scan Line [`VGA_CRTC_MAX_SCAN_LINE`] =
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
        self.crtc_regs[usize::from(VGA_CRTC_MAX_SCAN_LINE)] = VGA_CRTC_MAX_SCAN_LINE_DEFAULT;
        self.crtc_regs[usize::from(VGA_CRTC_OFFSET)] = VGA_CRTC_OFFSET_DEFAULT;
        self.crtc_regs[usize::from(VGA_CRTC_UNDERLINE_LOCATION)] =
            VGA_CRTC_UNDERLINE_LOCATION_DEFAULT;
        self.seq_index = 0;
        self.seq_regs = VGA_SEQ_DEFAULTS;
        self.gc_index = 0;
        self.gc_regs = VGA_GC_DEFAULTS;
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

    fn cell_offset(row: usize, col: usize) -> Option<usize> {
        if row >= VGA_TEXT_ROWS || col >= VGA_TEXT_COLS {
            return None;
        }
        Some((row * VGA_TEXT_COLS + col) * VGA_CELL_BYTES)
    }

    pub fn char_at(&self, row: usize, col: usize) -> Option<u8> {
        let off = Self::cell_offset(row, col)?;
        Some(self.mem[off])
    }

    pub fn attr_at(&self, row: usize, col: usize) -> Option<u8> {
        let off = Self::cell_offset(row, col)?;
        Some(self.mem[off + 1])
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: u8, attr: u8) -> bool {
        let Some(off) = Self::cell_offset(row, col) else {
            return false;
        };
        self.mem[off] = ch;
        self.mem[off + 1] = attr;
        true
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
        v.port_write(VGA_GC_INDEX, 1, 0x05); // Graphics Mode
        v.port_write(VGA_GC_DATA, 1, 0x00);
        v.port_write(VGA_GC_INDEX, 1, 0x08); // Bit Mask
        v.port_write(VGA_GC_DATA, 1, 0xAA);
        assert_eq!(v.gc_regs[0x05], 0x00);
        assert_eq!(v.gc_regs[0x08], 0xAA);
        assert_eq!(v.port_read(VGA_GC_INDEX, 1) as u8, 0x08);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xAA);
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x00);
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
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x10);
        v.port_write(VGA_GC_INDEX, 1, 0x08);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xFF);
    }

    #[test]
    fn graphics_controller_reset_defaults_mode03h() {
        // Spec: FreeVGA / OSDev / IBM VGA mode-03h-class GC programming SeaBIOS
        // probes — Mode `0x10`, Misc `0x0E`, Bit Mask `0xFF` (store/readback only).
        let v = VgaText::new();
        assert_eq!(v.gc_index, 0);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        assert_eq!(
            v.gc_regs,
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF]
        );
    }

    #[test]
    fn reset_restores_graphics_controller_defaults() {
        let mut v = VgaText::new();
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        v.port_write(VGA_GC_DATA, 1, 0x40);
        v.port_write(VGA_GC_INDEX, 1, 0x08);
        v.port_write(VGA_GC_DATA, 1, 0x00);
        v.reset();
        assert_eq!(v.gc_index, 0);
        assert_eq!(v.gc_regs, VGA_GC_DEFAULTS);
        v.port_write(VGA_GC_INDEX, 1, 0x05);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0x10);
        v.port_write(VGA_GC_INDEX, 1, 0x08);
        assert_eq!(v.port_read(VGA_GC_DATA, 1) as u8, 0xFF);
    }

    #[test]
    fn sequencer_index_data_round_trip() {
        // Spec: FreeVGA Sequencer Registers — write index 0x3C4, data 0x3C5;
        // indexes 0x00–0x04 (Reset, Clocking Mode, Map Mask, Character Map,
        // Memory Mode).
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, 0x02); // Map Mask
        v.port_write(VGA_SEQ_DATA, 1, 0x0F);
        v.port_write(VGA_SEQ_INDEX, 1, 0x04); // Memory Mode
        v.port_write(VGA_SEQ_DATA, 1, 0x06);
        assert_eq!(v.seq_regs[0x02], 0x0F);
        assert_eq!(v.seq_regs[0x04], 0x06);
        assert_eq!(v.port_read(VGA_SEQ_INDEX, 1) as u8, 0x04);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x06);
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x0F);
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
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x03);
    }

    #[test]
    fn sequencer_reset_defaults_mode03h() {
        // Spec: FreeVGA / IBM VGA mode-03h-class Sequencer programming SeaBIOS
        // probes — Reset `0x03`, Clocking Mode `0x00`, Map Mask `0x03`,
        // Character Map Select `0x00`, Memory Mode `0x02` (store/readback only).
        let v = VgaText::new();
        assert_eq!(v.seq_index, 0);
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        assert_eq!(v.seq_regs, [0x03, 0x00, 0x03, 0x00, 0x02]);
    }

    #[test]
    fn reset_restores_sequencer_defaults() {
        let mut v = VgaText::new();
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        v.port_write(VGA_SEQ_DATA, 1, 0x0F);
        v.port_write(VGA_SEQ_INDEX, 1, 0x01);
        v.port_write(VGA_SEQ_DATA, 1, 0x01);
        v.reset();
        assert_eq!(v.seq_index, 0);
        assert_eq!(v.seq_regs, VGA_SEQ_DEFAULTS);
        v.port_write(VGA_SEQ_INDEX, 1, 0x02);
        assert_eq!(v.port_read(VGA_SEQ_DATA, 1) as u8, 0x03);
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

    #[test]
    fn attribute_controller_flip_flop_index_data_round_trip() {
        // Spec: FreeVGA Accessing the Attribute Registers — read 0x3DA to reset
        // flip-flop; write index then data to 0x3C0; read data from 0x3C1.
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        assert!(!v.atc_flip_flop_data);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10); // Mode Control index, PAS=0
        assert!(v.atc_flip_flop_data);
        assert_eq!(v.atc_index, 0x10);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x0C);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs[0x10], 0x0C);
        // Read path: reset → write index → read 0x3C1 (does not toggle flip-flop).
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0C);
        assert!(v.atc_flip_flop_data); // still awaiting data after address write
        assert_eq!(v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8, 0x10);
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
        assert_eq!(v.atc_regs[0x10], 0x0C); // Mode Control default untouched
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
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, u32::from(VGA_ATC_PAS | 0x10));
        assert_eq!(v.atc_index, VGA_ATC_PAS | 0x10);
        assert_eq!(
            v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8,
            VGA_ATC_PAS | 0x10
        );
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0C);
        // Finishing the data write leaves PAS in the address register.
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x08);
        assert_eq!(v.atc_regs[0x10], 0x08);
        assert_eq!(
            v.port_read(VGA_ATC_ADDRESS_DATA, 1) as u8,
            VGA_ATC_PAS | 0x10
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
        // 00..05/14/07/38..3F, Mode Control 0x0C, plane enable 0x0F, pan 0x08.
        let v = VgaText::new();
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        assert_eq!(v.atc_regs[0x06], 0x14);
        assert_eq!(v.atc_regs[0x10], 0x0C);
        assert_eq!(v.atc_regs[0x12], 0x0F);
        assert_eq!(v.atc_regs[0x13], 0x08);
    }

    #[test]
    fn reset_restores_attribute_controller_defaults() {
        let mut v = VgaText::new();
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x00);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x06);
        assert_eq!(v.atc_regs[0x10], 0x00);
        assert_eq!(v.atc_regs[0x06], 0x06);
        v.reset();
        assert_eq!(v.atc_index, VGA_ATC_INDEX_DEFAULT);
        assert!(!v.atc_flip_flop_data);
        assert_eq!(v.atc_regs, VGA_ATC_DEFAULTS);
        let _ = v.port_read(VGA_INPUT_STATUS_1, 1);
        v.port_write(VGA_ATC_ADDRESS_DATA, 1, 0x10);
        assert_eq!(v.port_read(VGA_ATC_DATA_READ, 1) as u8, 0x0C);
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
        // programming via `0x3C9`. This stub stores the mask only (no host
        // render yet); verify `0x3C9` store/readback is unchanged under a
        // non-`0xFF` mask.
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
}
