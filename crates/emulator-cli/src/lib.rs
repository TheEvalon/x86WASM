//! Native CLI helpers for the x86WASM emulator runner.

use devices::{VgaRenderMode, VGA_TEXT_COLS, VGA_TEXT_ROWS};
use machine_pc::{
    build_hello_rom, GuestBootMeasure, GuestBootMedia, GuestOsMeasure, Machine, MachineError,
    PostReport, PostSpinConfig, PostTraceConfig, TracedPostReport, DEFAULT_POST_SPIN_WINDOW,
    DEFAULT_POST_TRACE_CAPACITY, EXPECTED_HELLO,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Default instruction budget when `--steps` is omitted.
pub const DEFAULT_MAX_STEPS: u64 = 100_000;

/// Default physical base for `--option-rom`.
///
/// Spec: IBM PC/AT memory map — the video option ROM region starts at
/// `0xC0000`, which is where a VGA BIOS (SeaVGABIOS included) is mapped.
pub const DEFAULT_OPTION_ROM_BASE: u64 = 0x000C_0000;

/// Option ROM size granularity. Spec: IBM PC option ROM header — the byte at
/// offset 2 counts 512-byte blocks.
pub const OPTION_ROM_BLOCK_BYTES: usize = 512;

const OPCODE_WINDOW_LEN: usize = 8;

/// Parsed CLI options (firmware path + run budget + diagnostics).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Lab / HELLO path — high map only via [`Machine::load_rom`].
    pub rom_path: Option<PathBuf>,
    /// Classic BIOS path — dual-map via [`Machine::load_bios_rom`].
    pub bios_path: Option<PathBuf>,
    pub max_steps: u64,
    /// Run the POST first-contact diagnostic instead of the normal run.
    pub post_probe: bool,
    /// Arm the bounded POST event trace. Implies [`Options::post_probe`].
    ///
    /// `None` leaves the probe untraced, which is what keeps the `--post-probe`
    /// output byte-identical to a build without this flag.
    pub post_trace: Option<PostTraceConfig>,
    /// Trailing program-counter window for the POST spin summary.
    ///
    /// Defaults to [`PostSpinConfig::default`]. `None` disables sampling
    /// (`--post-spin 0`). Implies [`Options::post_probe`] when set via the flag.
    pub post_spin: Option<PostSpinConfig>,
    /// Dump the 80×25 VGA text buffer after the run / probe.
    pub vga_text: bool,
    /// Render the current display through the VGA display fetch and report it.
    pub vga_frame: bool,
    /// Option ROM image to map before running (e.g. a VGA BIOS).
    pub option_rom_path: Option<PathBuf>,
    /// Physical base for [`Options::option_rom_path`].
    pub option_rom_base: u64,
    /// Raw IDE disk image for guest boot measure / attach.
    pub ide_image: Option<PathBuf>,
    /// Raw 1.44MB floppy image for guest boot measure / attach.
    pub floppy_image: Option<PathBuf>,
    /// Raw ATAPI/ISO CD-ROM image for El Torito guest measure.
    pub cdrom_image: Option<PathBuf>,
    /// Measure-first guest boot (load MBR/VBR/El Torito → entry, probe first stop).
    ///
    /// Does **not** claim FreeDOS/Linux boot success.
    pub guest_measure: bool,
    /// When [`Options::guest_measure`] is set with both IDE and floppy, prefer floppy.
    pub guest_floppy_first: bool,
    /// When [`Options::guest_measure`] is set, use El Torito no-emul handoff.
    pub guest_eltorito: bool,
    /// FreeDOS-*like* measure harness (synthetic fixture if no `--ide-image`).
    ///
    /// Does **not** claim a FreeDOS prompt.
    pub guest_freedos_measure: bool,
    /// Linux serial-path measure harness (synthetic stub if no `--ide-image`).
    ///
    /// Does **not** claim Linux boot or Milestone 2 exit.
    pub guest_linux_serial_measure: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rom_path: None,
            bios_path: None,
            max_steps: DEFAULT_MAX_STEPS,
            post_probe: false,
            post_trace: None,
            post_spin: Some(PostSpinConfig::default()),
            vga_text: false,
            vga_frame: false,
            option_rom_path: None,
            option_rom_base: DEFAULT_OPTION_ROM_BASE,
            ide_image: None,
            floppy_image: None,
            cdrom_image: None,
            guest_measure: false,
            guest_floppy_first: false,
            guest_eltorito: false,
            guest_freedos_measure: false,
            guest_linux_serial_measure: false,
        }
    }
}

/// Decoded PC option ROM header for an image the CLI mapped.
///
/// Spec: IBM PC option ROM convention — `55 AA` signature, a 512-byte block
/// count at offset 2, and a whole-image checksum of zero modulo 256 over the
/// declared extent. The same rules `firmware/build-scripts/check-option-rom.py`
/// applies to a fresh SeaVGABIOS build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionRomInfo {
    /// Physical base the image was mapped at.
    pub base: u64,
    /// Length of the file on disk.
    pub size: usize,
    /// `55 AA` present at offset 0.
    pub signature_ok: bool,
    /// Block count byte at offset 2 (`None` when the image is too short).
    pub blocks: Option<u8>,
    /// `blocks * 512`, the extent the header declares.
    pub declared_bytes: Option<usize>,
    /// Checksum over the declared extent; `None` when that extent does not fit.
    pub checksum: Option<u8>,
}

impl OptionRomInfo {
    /// True when the header is well formed and the declared extent sums to zero.
    pub fn is_valid(&self) -> bool {
        self.signature_ok
            && self
                .declared_bytes
                .is_some_and(|d| d != 0 && d <= self.size)
            && self.checksum == Some(0)
    }
}

impl std::fmt::Display for OptionRomInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "option-rom: base=0x{:08X} size={} signature={} blocks=",
            self.base,
            self.size,
            if self.signature_ok { "55AA" } else { "missing" }
        )?;
        match self.blocks {
            Some(blocks) => write!(f, "{blocks}")?,
            None => f.write_str("none")?,
        }
        f.write_str(" declared=")?;
        match self.declared_bytes {
            Some(declared) => write!(f, "{declared}")?,
            None => f.write_str("none")?,
        }
        f.write_str(" checksum=")?;
        match self.checksum {
            Some(sum) => write!(f, "0x{sum:02X}")?,
            None => f.write_str("none")?,
        }
        write!(
            f,
            " status={}",
            if self.is_valid() { "ok" } else { "invalid" }
        )
    }
}

/// Decode a PC option ROM header without mapping anything.
pub fn describe_option_rom(base: u64, data: &[u8]) -> OptionRomInfo {
    let signature_ok = data.len() >= 2 && data[0] == 0x55 && data[1] == 0xAA;
    let blocks = data.get(2).copied();
    let declared_bytes = blocks.map(|b| usize::from(b) * OPTION_ROM_BLOCK_BYTES);
    let checksum = declared_bytes.and_then(|declared| {
        if declared == 0 || declared > data.len() {
            None
        } else {
            Some(
                data[..declared]
                    .iter()
                    .fold(0u8, |acc, b| acc.wrapping_add(*b)),
            )
        }
    });
    OptionRomInfo {
        base,
        size: data.len(),
        signature_ok,
        blocks,
        declared_bytes,
        checksum,
    }
}

/// A built machine plus what was installed into it.
pub struct BuiltMachine {
    pub machine: Machine,
    pub kind: FirmwareKind,
    /// Present when `--option-rom` mapped an image.
    pub option_rom: Option<OptionRomInfo>,
}

/// Result of parsing argv (excluding program name).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // `Options` carries several PathBufs for CLI media/firmware.
pub enum ParsedArgs {
    Help,
    Run(Options),
}

/// Which firmware image was installed before `reset` / `run`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareKind {
    /// Built-in HELLO ROM (`--rom` omitted and `--bios` omitted).
    Hello,
    /// Explicit `--rom` image (high map only).
    Rom,
    /// Explicit `--bios` image (top-of-4 GiB + below-1 MiB alias).
    Bios,
}

/// Deterministic architectural context captured after an execution step fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuFailureContext {
    pub completed_steps: u64,
    pub cs: u16,
    pub ip: u16,
    pub rip: u64,
    pub linear_pc: u64,
    pub opcode_bytes: [Option<u8>; OPCODE_WINDOW_LEN],
}

impl std::fmt::Display for CpuFailureContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "completed_steps={} cs:ip={:04X}:{:04X} rip=0x{:016X} \
             linear_pc=0x{:016X} opcode_bytes=[",
            self.completed_steps, self.cs, self.ip, self.rip, self.linear_pc
        )?;
        for (index, byte) in self.opcode_bytes.iter().enumerate() {
            if index != 0 {
                f.write_str(" ")?;
            }
            match byte {
                Some(byte) => write!(f, "{byte:02X}")?,
                None => f.write_str("??")?,
            }
        }
        f.write_str("]")
    }
}

/// Execution failure with its original machine error retained as the source.
#[derive(Clone, Debug)]
pub struct ExecutionFailure {
    pub context: CpuFailureContext,
    source: Arc<MachineError>,
}

impl ExecutionFailure {
    fn new(context: CpuFailureContext, source: MachineError) -> Self {
        Self {
            context,
            source: Arc::new(source),
        }
    }

    pub fn source_error(&self) -> &MachineError {
        self.source.as_ref()
    }
}

impl PartialEq for ExecutionFailure {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && self.source.to_string() == other.source.to_string()
    }
}

impl Eq for ExecutionFailure {}

/// CLI argument / usage errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliError {
    UnknownArgument(String),
    MissingValue(&'static str),
    InvalidSteps(String),
    InvalidAddress(String),
    RomAndBios,
    GuestMeasureNeedsImage,
    Io(String),
    Machine(String),
    Execution(ExecutionFailure),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownArgument(a) => write!(f, "Unknown argument: {a}"),
            Self::MissingValue(flag) => write!(f, "Missing value for {flag}"),
            Self::InvalidSteps(v) => write!(f, "Invalid --steps value: {v}"),
            Self::InvalidAddress(v) => write!(f, "Invalid --option-rom-base value: {v}"),
            Self::RomAndBios => write!(f, "Use only one of --rom or --bios"),
            Self::GuestMeasureNeedsImage => write!(
                f,
                "--guest-measure requires --ide-image, --floppy-image, and/or --cdrom-image"
            ),
            Self::Io(msg) => write!(f, "{msg}"),
            Self::Machine(msg) => write!(f, "{msg}"),
            Self::Execution(failure) => write!(
                f,
                "Execution error: {} error={}",
                failure.context,
                failure.source_error()
            ),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Execution(failure) => Some(failure.source_error()),
            _ => None,
        }
    }
}

/// Usage / help text for `--help` / `-h`.
pub fn usage() -> String {
    format!(
        "Usage: emulator-cli [--rom path.bin | --bios path.bin] [--steps N] [--post-probe]\n\
         \x20                  [--post-trace [N]] [--post-spin [N]]\n\
         \x20                  [--option-rom path.bin [--option-rom-base ADDR]]\n\
         \x20                  [--ide-image path.bin] [--floppy-image path.img]\n\
         \x20                  [--cdrom-image path.iso]\n\
         \x20                  [--guest-measure [--guest-floppy-first|--guest-eltorito]]\n\
         \x20                  [--guest-freedos-measure] [--guest-linux-serial-measure]\n\
         \x20                  [--vga-text] [--vga-frame]\n\
         --rom              Load a lab ROM at top-of-4GiB only (HELLO-style).\n\
         --bios             Load a legacy BIOS via dual map (top-of-4GiB + below-1MiB alias).\n\
         --post-probe       Report POST first contact (first failure, unclaimed ports,\n\
         \x20                  unmapped MMIO) instead of validating the run.\n\
         --post-trace       Implies --post-probe and appends a bounded trace of the most\n\
         \x20                  recent platform accesses (port I/O, PCI config cycles, PAM\n\
         \x20                  programming, VGA aperture, memory faults). Optional N is the\n\
         \x20                  event capacity (default {DEFAULT_POST_TRACE_CAPACITY}). The\n\
         \x20                  --post-probe lines above it are unchanged.\n\
         --post-spin        Implies --post-probe. Size of the trailing program-counter\n\
         \x20                  window used for the spin summary (default {DEFAULT_POST_SPIN_WINDOW};\n\
         \x20                  0 disables it). The --post-probe header line is unchanged.\n\
         --option-rom       Map an option ROM image (e.g. a VGA BIOS) and report its\n\
         \x20                  55AA/size/checksum header. Mapping only: nothing scans or\n\
         \x20                  executes option ROMs yet.\n\
         --option-rom-base  Physical base for --option-rom (default 0x{DEFAULT_OPTION_ROM_BASE:05X}).\n\
         --ide-image        Attach a raw IDE disk image (primary master).\n\
         --floppy-image     Attach a raw 1.44MB floppy image.\n\
         --cdrom-image      Attach a raw ATAPI/ISO CD-ROM image (El Torito measure).\n\
         --guest-measure    Load boot image and report the first stop reason (v2 harness:\n\
         \x20                  checkpoints + serial capture; not a FreeDOS/Linux success claim).\n\
         \x20                  Requires --ide-image, --floppy-image, and/or --cdrom-image.\n\
         --guest-floppy-first  With --guest-measure, force floppy CHS (0,0,1) handoff.\n\
         --guest-eltorito   With --guest-measure, force El Torito no-emul handoff\n\
         \x20                  (requires --cdrom-image).\n\
         --guest-freedos-measure  Measure FreeDOS-*like* synthetic MBR+payload (or\n\
         \x20                  --ide-image). Reports serial/VGA/checkpoints; does NOT\n\
         \x20                  claim a FreeDOS prompt.\n\
         --guest-linux-serial-measure  Measure Linux serial-path stub (or --ide-image).\n\
         \x20                  Captures COM1; documents gaps; does NOT claim Linux boot\n\
         \x20                  or Milestone 2 exit.\n\
         --vga-text         Dump the {VGA_TEXT_COLS}x{VGA_TEXT_ROWS} VGA text buffer after the run.\n\
         --vga-frame        Render the display through the VGA display fetch and report the\n\
         \x20                  frame geometry, RGBA size, and whether a font is installed.\n\
         \x20                  Text mode 03h, mode 13h, and planar 16-color (0Dh/0Eh/10h/12h)\n\
         \x20                  have a renderer; any other programming is reported as having\n\
         \x20                  none rather than rendered.\n\
         Diagnostics are appended after the normal / --post-probe output, which is unchanged.\n\
         Default ROM prints '{EXPECTED_HELLO}' via COM1 and port 0x402."
    )
}

/// Parse a decimal or `0x`-prefixed physical address.
fn parse_address(value: &str) -> Result<u64, CliError> {
    let trimmed = value.trim();
    let parsed = match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => trimmed.parse(),
    };
    parsed.map_err(|_| CliError::InvalidAddress(value.to_string()))
}

/// Parse CLI arguments (excluding argv[0]).
pub fn parse_args<I, S>(args: I) -> Result<ParsedArgs, CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut opts = Options::default();
    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        let arg = arg.as_ref();
        match arg {
            "--help" | "-h" => return Ok(ParsedArgs::Help),
            "--rom" => {
                let path = iter.next().ok_or(CliError::MissingValue("--rom"))?;
                opts.rom_path = Some(PathBuf::from(path.as_ref()));
            }
            "--bios" => {
                let path = iter.next().ok_or(CliError::MissingValue("--bios"))?;
                opts.bios_path = Some(PathBuf::from(path.as_ref()));
            }
            "--post-probe" => opts.post_probe = true,
            // The capacity is optional: `--post-trace` alone keeps
            // DEFAULT_POST_TRACE_CAPACITY events. Only a value that parses as a
            // count is consumed, so `--post-trace --steps 100` still works.
            "--post-trace" => {
                let capacity = match iter.peek().and_then(|v| v.as_ref().parse::<usize>().ok()) {
                    Some(capacity) => {
                        iter.next();
                        capacity
                    }
                    None => DEFAULT_POST_TRACE_CAPACITY,
                };
                opts.post_probe = true;
                opts.post_trace = Some(PostTraceConfig::with_capacity(capacity));
            }
            // Optional window: `--post-spin` alone keeps the default; `--post-spin 0`
            // disables sampling; any other N sizes the window.
            "--post-spin" => {
                let window = match iter.peek().and_then(|v| v.as_ref().parse::<usize>().ok()) {
                    Some(window) => {
                        iter.next();
                        window
                    }
                    None => DEFAULT_POST_SPIN_WINDOW,
                };
                opts.post_probe = true;
                opts.post_spin = if window == 0 {
                    None
                } else {
                    Some(PostSpinConfig::with_window(window))
                };
            }
            "--vga-text" => opts.vga_text = true,
            "--vga-frame" => opts.vga_frame = true,
            "--option-rom" => {
                let path = iter.next().ok_or(CliError::MissingValue("--option-rom"))?;
                opts.option_rom_path = Some(PathBuf::from(path.as_ref()));
            }
            "--option-rom-base" => {
                let v = iter
                    .next()
                    .ok_or(CliError::MissingValue("--option-rom-base"))?;
                opts.option_rom_base = parse_address(v.as_ref())?;
            }
            "--ide-image" => {
                let path = iter.next().ok_or(CliError::MissingValue("--ide-image"))?;
                opts.ide_image = Some(PathBuf::from(path.as_ref()));
            }
            "--floppy-image" => {
                let path = iter
                    .next()
                    .ok_or(CliError::MissingValue("--floppy-image"))?;
                opts.floppy_image = Some(PathBuf::from(path.as_ref()));
            }
            "--cdrom-image" => {
                let path = iter.next().ok_or(CliError::MissingValue("--cdrom-image"))?;
                opts.cdrom_image = Some(PathBuf::from(path.as_ref()));
            }
            "--guest-measure" => opts.guest_measure = true,
            "--guest-floppy-first" => opts.guest_floppy_first = true,
            "--guest-eltorito" => opts.guest_eltorito = true,
            "--guest-freedos-measure" => opts.guest_freedos_measure = true,
            "--guest-linux-serial-measure" => opts.guest_linux_serial_measure = true,
            "--steps" => {
                let v = iter.next().ok_or(CliError::MissingValue("--steps"))?;
                opts.max_steps = v
                    .as_ref()
                    .parse()
                    .map_err(|_| CliError::InvalidSteps(v.as_ref().to_string()))?;
            }
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }
    if opts.rom_path.is_some() && opts.bios_path.is_some() {
        return Err(CliError::RomAndBios);
    }
    if opts.guest_measure
        && opts.ide_image.is_none()
        && opts.floppy_image.is_none()
        && opts.cdrom_image.is_none()
    {
        return Err(CliError::GuestMeasureNeedsImage);
    }
    if opts.guest_eltorito && opts.cdrom_image.is_none() {
        return Err(CliError::GuestMeasureNeedsImage);
    }
    Ok(ParsedArgs::Run(opts))
}

/// Build a machine and install firmware from `opts`.
///
/// `--bios` uses [`Machine::with_bios_rom`] (→ [`Machine::load_bios_rom`]).
/// `--rom` / default HELLO use [`Machine::load_rom`].
///
/// `--option-rom` appends a further ROM window via `PhysMem::add_rom`, which
/// leaves the firmware windows in place and survives [`Machine::reset`]. An
/// image with a malformed header is still mapped, and reported as `invalid`,
/// because inspecting a broken ROM is a legitimate bring-up step.
pub fn build_machine(opts: &Options) -> Result<BuiltMachine, CliError> {
    const RAM: usize = 16 * 1024 * 1024;
    let (mut machine, kind) = if let Some(path) = &opts.bios_path {
        let data = read_file(path)?;
        (
            Machine::with_bios_rom(RAM, &data).map_err(machine_err)?,
            FirmwareKind::Bios,
        )
    } else {
        let mut machine = Machine::new(RAM);
        if let Some(path) = &opts.rom_path {
            let data = read_file(path)?;
            machine.load_rom(&data).map_err(machine_err)?;
            (machine, FirmwareKind::Rom)
        } else {
            machine.load_rom(&build_hello_rom()).map_err(machine_err)?;
            (machine, FirmwareKind::Hello)
        }
    };

    let option_rom = match &opts.option_rom_path {
        Some(path) => {
            let data = read_file(path)?;
            let info = describe_option_rom(opts.option_rom_base, &data);
            machine.mem.add_rom(opts.option_rom_base, data);
            Some(info)
        }
        None => None,
    };

    if let Some(path) = &opts.ide_image {
        let data = read_file(path)?;
        machine.attach_ide_image(data);
    }
    if let Some(path) = &opts.floppy_image {
        let data = read_file(path)?;
        machine
            .attach_floppy_image(data)
            .map_err(|e| CliError::Machine(format!("Failed to attach floppy: {e}")))?;
    }
    if let Some(path) = &opts.cdrom_image {
        let data = read_file(path)?;
        machine.attach_atapi_cdrom_image(data);
    }

    Ok(BuiltMachine {
        machine,
        kind,
        option_rom,
    })
}

/// Media policy for [`run_guest_measure`].
pub fn guest_boot_media(opts: &Options) -> GuestBootMedia {
    if opts.guest_eltorito
        || (opts.cdrom_image.is_some() && opts.ide_image.is_none() && opts.floppy_image.is_none())
    {
        GuestBootMedia::ElTorito
    } else if opts.guest_floppy_first || (opts.floppy_image.is_some() && opts.ide_image.is_none()) {
        GuestBootMedia::FloppyFirst
    } else {
        GuestBootMedia::IdePrefer
    }
}

/// Load boot sector to `0x7C00` and measure the first stop (not boot success).
pub fn run_guest_measure(
    machine: &mut Machine,
    media: GuestBootMedia,
    max_steps: u64,
) -> Result<GuestBootMeasure, CliError> {
    machine
        .measure_guest_boot(media, max_steps)
        .map_err(|e| CliError::Machine(format!("Guest measure setup failed: {e}")))
}

/// FreeDOS-*like* measure (synthetic fixture when IDE empty). Not a prompt claim.
pub fn run_freedos_measure(
    machine: &mut Machine,
    max_steps: u64,
) -> Result<GuestOsMeasure, CliError> {
    machine
        .measure_freedos_like(max_steps)
        .map_err(|e| CliError::Machine(format!("FreeDOS-like measure setup failed: {e}")))
}

/// Linux serial-path measure (synthetic stub when IDE empty). Not M2 exit.
pub fn run_linux_serial_measure(
    machine: &mut Machine,
    max_steps: u64,
) -> Result<GuestOsMeasure, CliError> {
    machine
        .measure_linux_serial_path(max_steps)
        .map_err(|e| CliError::Machine(format!("Linux serial measure setup failed: {e}")))
}

/// Render the 80×25 VGA text buffer as lines of text.
///
/// Bytes outside printable ASCII (`0x20`–`0x7E`) render as `.`; there is no
/// CP437 glyph translation and attributes are not shown. Rows are bracketed by
/// `|` so trailing spaces stay visible. The viewport follows the CRTC Start
/// Address and Offset the guest programmed, matching `VgaText::char_at`.
pub fn vga_text_dump(machine: &Machine) -> String {
    let vga = &machine.vga;
    let (cursor_row, cursor_col) = vga.crtc_cursor_row_col();
    let mut rows = Vec::with_capacity(VGA_TEXT_ROWS);
    let mut nonblank_rows = 0;

    for row in 0..VGA_TEXT_ROWS {
        let mut line = String::with_capacity(VGA_TEXT_COLS);
        let mut blank = true;
        for col in 0..VGA_TEXT_COLS {
            let ch = vga.char_at(row, col).unwrap_or(0);
            if ch != b' ' && ch != 0 {
                blank = false;
            }
            line.push(if (0x20..=0x7E).contains(&ch) {
                ch as char
            } else {
                '.'
            });
        }
        if !blank {
            nonblank_rows += 1;
        }
        rows.push(line);
    }

    let mut out = format!(
        "vga-text: cols={} rows={} cursor=({},{}) start=0x{:04X} pitch={} nonblank_rows={}",
        VGA_TEXT_COLS,
        VGA_TEXT_ROWS,
        cursor_row,
        cursor_col,
        vga.text_start_address(),
        vga.text_row_pitch_chars(),
        nonblank_rows
    );
    for (index, line) in rows.iter().enumerate() {
        out.push_str(&format!("\n{index:02} |{line}|"));
    }
    out
}

fn read_file(path: &Path) -> Result<Vec<u8>, CliError> {
    fs::read(path).map_err(|e| CliError::Io(format!("Failed to read {}: {e}", path.display())))
}

fn machine_err(e: MachineError) -> CliError {
    CliError::Machine(format!("Failed to load firmware: {e}"))
}

/// Capture the same real-mode fetch addresses used by the interpreter.
///
/// Intel SDM Vol. 3 §3.4.2: a real-mode linear address is the cached segment
/// base plus the 16-bit offset. Instruction fetch wraps IP at 16 bits; the
/// cached reset CS base remains `0xFFFF_0000`.
fn capture_cpu_failure_context(machine: &Machine, completed_steps: u64) -> CpuFailureContext {
    let rip = machine.cpu.rip;
    let ip = rip as u16;
    let cs_base = machine.cpu.cs.base;
    let linear_pc = cs_base.wrapping_add(u64::from(ip));
    let opcode_bytes = std::array::from_fn(|index| {
        let offset = ip.wrapping_add(index as u16);
        let address = cs_base.wrapping_add(u64::from(offset));
        machine.mem.read_u8(address).ok()
    });
    CpuFailureContext {
        completed_steps,
        cs: machine.cpu.cs.selector,
        ip,
        rip,
        linear_pc,
        opcode_bytes,
    }
}

/// Run until HLT / step budget; validate HELLO output when applicable.
pub fn run_machine(
    machine: &mut Machine,
    kind: FirmwareKind,
    max_steps: u64,
) -> Result<(u64, String, String), CliError> {
    machine.reset();
    let mut steps = 0;
    while steps < max_steps && !machine.cpu.halted {
        if let Err(source) = machine.step() {
            let context = capture_cpu_failure_context(machine, steps);
            return Err(CliError::Execution(ExecutionFailure::new(context, source)));
        }
        steps += 1;
    }
    let com1 = machine.com1_text();
    let dbg = machine.debug_text();
    if kind == FirmwareKind::Hello && (com1 != EXPECTED_HELLO || dbg != EXPECTED_HELLO) {
        return Err(CliError::Machine("HELLO ROM output mismatch".to_string()));
    }
    Ok((steps, com1, dbg))
}

/// Run the POST first-contact diagnostic and return its structured report.
///
/// Unlike [`run_machine`] this makes no claim about success: it reports how far
/// the firmware got and what stopped it. Resets the machine first so the run
/// starts from the architectural reset state.
pub fn run_post_probe(machine: &mut Machine, max_steps: u64) -> PostReport {
    machine.reset();
    machine.probe_post(max_steps)
}

/// [`run_post_probe`] with optional bounded event trace and spin summary.
///
/// With `trace` `None` and default spin the printed report header stays
/// byte-identical to [`run_post_probe`]; the spin block is additional output
/// for halt / step-budget stops only.
pub fn run_post_probe_traced(
    machine: &mut Machine,
    max_steps: u64,
    trace: Option<PostTraceConfig>,
) -> TracedPostReport {
    run_post_probe_options(machine, max_steps, trace, Some(PostSpinConfig::default()))
}

/// Full POST-probe wiring used by the CLI (`--post-trace` / `--post-spin`).
pub fn run_post_probe_options(
    machine: &mut Machine,
    max_steps: u64,
    trace: Option<PostTraceConfig>,
    spin: Option<PostSpinConfig>,
) -> TracedPostReport {
    machine.reset();
    machine.probe_post_options(max_steps, trace, spin)
}

/// Render the current display and describe the result.
///
/// Only three programmings have a display fetch in this model — alphanumeric
/// (text mode 03h), chain-4 256-color (mode 13h) and planar 16-color (modes
/// 0Dh/0Eh/10h/12h). Anything else reports that there is no renderer instead of
/// producing a frame that is not what the hardware would show.
/// `blink_off_half` selects the invisible half of the text blink cycle; the
/// caller owns the phase because there is no retrace timer.
pub fn vga_frame_report(machine: &Machine, blink_off_half: bool) -> String {
    let vga = &machine.vga;
    let mode = vga.render_mode();
    let Some(frame) = vga.render_frame(blink_off_half) else {
        return format!(
            "vga-frame: mode={} rendered=no — the current programming has no display fetch in \
             this model (no CGA-compatible modes, no unchained mode X, no VBE, no host display)",
            render_mode_name(mode)
        );
    };
    let rgba = vga.frame_rgba8(&frame);
    let nonzero = frame.pixels.iter().filter(|index| **index != 0).count();
    let font = match frame.font_installed {
        Some(true) => "yes",
        Some(false) => "no",
        None => "n/a",
    };
    format!(
        "vga-frame: mode={} rendered=yes width={} height={} pixels={} nonzero_indices={} \
         rgba_bytes={} blink_off_half={} font_installed={}",
        render_mode_name(frame.mode),
        frame.width,
        frame.height,
        frame.pixels.len(),
        nonzero,
        rgba.len(),
        u8::from(blink_off_half),
        font,
    )
}

fn render_mode_name(mode: VgaRenderMode) -> &'static str {
    match mode {
        VgaRenderMode::Text => "text",
        VgaRenderMode::Graphics256Chain4 => "graphics256-chain4",
        VgaRenderMode::Graphics16Planar => "graphics16-planar",
        VgaRenderMode::Unsupported => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::PortDevice;
    use std::io::Write;

    /// Tiny synthetic BIOS (not SeaBIOS): HLT at Intel reset vector offset.
    ///
    /// Spec: reset at phys `0xFFFFFFF0`; image is right-aligned under 4 GiB, so
    /// byte `len - 16` maps there. Also dual-maps below 1 MiB via `load_bios_rom`.
    fn tiny_bios_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 256];
        rom[0] = 0xEA; // marker at image start (low + high)
        rom[256 - 16] = 0xF4; // HLT at reset vector
        rom[255] = 0x55;
        rom
    }

    /// NOP followed by valid-but-unimplemented WAIT at the Intel reset vector.
    fn failing_bios_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 256];
        let reset = rom.len() - 16;
        rom[reset..reset + 9]
            .copy_from_slice(&[0x90, 0x9B, 0xF4, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        rom
    }

    struct TempBios {
        path: PathBuf,
        data: Vec<u8>,
    }

    impl TempBios {
        fn create() -> Self {
            let data = tiny_bios_rom();
            let path = std::env::temp_dir().join(format!(
                "x86wasm-cli-bios-{}-{}.bin",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let mut f = fs::File::create(&path).expect("create temp bios");
            f.write_all(&data).expect("write temp bios");
            Self { path, data }
        }
    }

    impl Drop for TempBios {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn usage_mentions_bios_flag() {
        let u = usage();
        assert!(u.contains("--bios"), "{u}");
        assert!(u.contains("--rom"), "{u}");
    }

    #[test]
    fn parse_bios_path_and_steps() {
        let parsed = parse_args(["--bios", "fw.bin", "--steps", "42"]).unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                bios_path: Some(PathBuf::from("fw.bin")),
                max_steps: 42,
                ..Options::default()
            })
        );
    }

    #[test]
    fn parse_post_probe_flag() {
        let parsed = parse_args(["--bios", "fw.bin", "--post-probe"]).unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                bios_path: Some(PathBuf::from("fw.bin")),
                post_probe: true,
                ..Options::default()
            })
        );
        assert!(usage().contains("--post-probe"));
    }

    /// `--post-probe` reports first contact instead of validating output.
    #[test]
    fn post_probe_reports_first_unsupported_opcode() {
        let mut machine =
            Machine::with_bios_rom(16 * 1024 * 1024, &failing_bios_rom()).expect("load BIOS");

        let report = run_post_probe(&mut machine, 16);

        let failure = report.failure().expect("first failure");
        assert_eq!(
            failure.kind,
            machine_pc::PostFailureKind::UnsupportedOpcode(0x9B)
        );
        assert_eq!(failure.opcode_bytes[0], Some(0x9B));
        assert_eq!(report.steps, 1);
    }

    /// `--post-trace` implies `--post-probe` and defaults its capacity.
    #[test]
    fn parse_post_trace_implies_post_probe_with_a_default_capacity() {
        let parsed = parse_args(["--bios", "fw.bin", "--post-trace"]).unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                bios_path: Some(PathBuf::from("fw.bin")),
                post_probe: true,
                post_trace: Some(PostTraceConfig::with_capacity(DEFAULT_POST_TRACE_CAPACITY)),
                ..Options::default()
            })
        );
        assert!(usage().contains("--post-trace"));
    }

    /// An explicit capacity is consumed; a following flag is not.
    #[test]
    fn parse_post_trace_capacity_is_optional() {
        let ParsedArgs::Run(opts) = parse_args(["--post-trace", "16"]).unwrap() else {
            panic!("expected a run");
        };
        assert_eq!(opts.post_trace, Some(PostTraceConfig::with_capacity(16)));
        assert_eq!(opts.max_steps, DEFAULT_MAX_STEPS);

        let ParsedArgs::Run(opts) = parse_args(["--post-trace", "--steps", "99"]).unwrap() else {
            panic!("expected a run");
        };
        assert_eq!(
            opts.post_trace,
            Some(PostTraceConfig::with_capacity(DEFAULT_POST_TRACE_CAPACITY))
        );
        assert_eq!(opts.max_steps, 99);
    }

    /// The probe output is unchanged by the flag's existence: an untraced run
    /// prints exactly what `--post-probe` printed before the trace existed.
    #[test]
    fn untraced_probe_output_is_byte_identical() {
        let mut plain =
            Machine::with_bios_rom(16 * 1024 * 1024, &failing_bios_rom()).expect("load BIOS");
        let mut traced =
            Machine::with_bios_rom(16 * 1024 * 1024, &failing_bios_rom()).expect("load BIOS");

        let expected = run_post_probe(&mut plain, 16).to_string();
        let untraced = run_post_probe_traced(&mut traced, 16, None);

        assert!(untraced.trace.is_none());
        assert_eq!(untraced.to_string(), expected);
    }

    /// With a trace armed the same first lines are followed by a trace section.
    #[test]
    fn traced_probe_appends_a_trace_section_after_the_same_report() {
        let mut plain =
            Machine::with_bios_rom(16 * 1024 * 1024, &failing_bios_rom()).expect("load BIOS");
        let mut traced =
            Machine::with_bios_rom(16 * 1024 * 1024, &failing_bios_rom()).expect("load BIOS");

        let expected = run_post_probe(&mut plain, 16).to_string();
        let report =
            run_post_probe_traced(&mut traced, 16, Some(PostTraceConfig::with_capacity(8)));

        let text = report.to_string();
        assert!(text.starts_with(&expected), "{text}");
        assert!(text.contains("post-trace: events="), "{text}");
    }

    /// A reset machine is in the mode-03h text programming, which renders.
    #[test]
    fn vga_frame_report_describes_the_text_frame() {
        let built = build_machine(&Options::default()).expect("build HELLO");

        let line = vga_frame_report(&built.machine, false);

        assert!(line.contains("mode=text"), "{line}");
        assert!(line.contains("rendered=yes"), "{line}");
        assert!(line.contains("width=720 height=400"), "{line}");
        assert!(line.contains("pixels=288000"), "{line}");
        assert!(line.contains("rgba_bytes=1152000"), "{line}");
        assert!(line.contains("font_installed=no"), "{line}");
    }

    /// Reset installs no font, so the only non-background pixels in the text
    /// frame come from the hardware cursor, not from glyphs. Anything more
    /// would mean a glyph was rendered from a character generator that has no
    /// font loaded.
    #[test]
    fn vga_frame_report_shows_only_the_cursor_at_reset() {
        let built = build_machine(&Options::default()).expect("build HELLO");
        let frame = built
            .machine
            .vga
            .render_frame(false)
            .expect("text mode renders");

        let nonzero = frame.pixels.iter().filter(|index| **index != 0).count();
        let cell_pixels =
            built.machine.vga.text_cell_width() * built.machine.vga.text_cell_height();
        assert!(nonzero > 0, "the reset cursor should be visible");
        assert!(
            nonzero <= cell_pixels,
            "no font is loaded, so nothing outside the single cursor cell should be lit: \
             {nonzero} of {cell_pixels}"
        );
    }

    /// Graphics programming with no renderer is reported honestly instead of
    /// producing a frame this model cannot actually fetch.
    #[test]
    fn vga_frame_report_admits_when_there_is_no_renderer() {
        use devices::{VGA_GC_INDEX, VGA_GC_MISC, VGA_GC_MISC_DEFAULT, VGA_GC_MISC_GRAPHICS_MODE};

        let mut built = build_machine(&Options::default()).expect("build HELLO");
        let vga = &mut built.machine.vga;
        vga.port_write(VGA_GC_INDEX, 1, u32::from(VGA_GC_MISC));
        vga.port_write(
            VGA_GC_INDEX + 1,
            1,
            u32::from(VGA_GC_MISC_DEFAULT | VGA_GC_MISC_GRAPHICS_MODE),
        );

        let line = vga_frame_report(&built.machine, false);

        assert!(line.contains("mode=unsupported"), "{line}");
        assert!(line.contains("rendered=no"), "{line}");
        assert!(line.contains("no CGA-compatible modes"), "{line}");
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse_args(["--help"]).unwrap(), ParsedArgs::Help);
        assert_eq!(parse_args(["-h"]).unwrap(), ParsedArgs::Help);
    }

    #[test]
    fn parse_rejects_rom_and_bios_together() {
        assert_eq!(
            parse_args(["--rom", "a.bin", "--bios", "b.bin"]),
            Err(CliError::RomAndBios)
        );
    }

    #[test]
    fn parse_rejects_missing_bios_value() {
        assert_eq!(
            parse_args(["--bios"]),
            Err(CliError::MissingValue("--bios"))
        );
    }

    /// Integration-style: temp tiny ROM via `--bios` → `with_bios_rom` dual map + HLT.
    #[test]
    fn bios_flag_loads_temp_tiny_rom_and_halts() {
        let temp = TempBios::create();
        let opts = Options {
            bios_path: Some(temp.path.clone()),
            ..Options::default()
        };
        let BuiltMachine {
            mut machine, kind, ..
        } = build_machine(&opts).expect("build with --bios");
        assert_eq!(kind, FirmwareKind::Bios);

        // Dual map: image start marker at high base and low alias.
        let high_base = 0x1_0000_0000u64 - temp.data.len() as u64;
        let low_base = 0x0010_0000u64 - temp.data.len() as u64;
        assert_eq!(machine.mem.read_u8(high_base).unwrap(), 0xEA);
        assert_eq!(machine.mem.read_u8(low_base).unwrap(), 0xEA);
        assert_eq!(machine.mem.read_u8(0xFFFF_FFF0).unwrap(), 0xF4);
        assert_eq!(
            machine
                .mem
                .read_u8(low_base + (temp.data.len() as u64 - 16))
                .unwrap(),
            0xF4
        );

        let (steps, _, _) = run_machine(&mut machine, kind, 16).expect("run");
        assert!(machine.cpu.halted, "tiny BIOS should HLT at reset vector");
        assert!(steps >= 1);
    }

    #[test]
    fn bios_execution_error_reports_deterministic_cpu_context() {
        let mut machine =
            Machine::with_bios_rom(16 * 1024 * 1024, &failing_bios_rom()).expect("load BIOS");

        let err = run_machine(&mut machine, FirmwareKind::Bios, 16).unwrap_err();

        assert_eq!(
            err.to_string(),
            "Execution error: completed_steps=1 cs:ip=F000:FFF1 \
             rip=0x000000000000FFF1 linear_pc=0x00000000FFFFFFF1 \
             opcode_bytes=[9B F4 11 22 33 44 55 66] \
             error=unsupported opcode 0x9B"
        );
        let source = std::error::Error::source(&err).expect("original execution error source");
        assert_eq!(source.to_string(), "unsupported opcode 0x9B");
        assert!(
            source.downcast_ref::<MachineError>().is_some(),
            "source should retain the original MachineError"
        );
    }

    #[test]
    fn failure_context_is_safe_at_top_of_address_space() {
        let mut machine = Machine::new(8);
        machine.mem.map_rom(u64::MAX - 1, vec![0xAA, 0xBB]);
        for (addr, byte) in [0xCC, 0xDD, 0xEE, 0xF0, 0x12, 0x34].into_iter().enumerate() {
            machine.mem.write_u8(addr as u64, byte).unwrap();
        }
        machine.cpu.cs.base = u64::MAX - 1;
        machine.cpu.rip = 0;

        let context = capture_cpu_failure_context(&machine, 7);

        assert_eq!(context.completed_steps, 7);
        assert_eq!(context.linear_pc, u64::MAX - 1);
        assert_eq!(
            context.opcode_bytes,
            [
                Some(0xAA),
                Some(0xBB),
                Some(0xCC),
                Some(0xDD),
                Some(0xEE),
                Some(0xF0),
                Some(0x12),
                Some(0x34),
            ]
        );
    }

    #[test]
    fn failure_context_wraps_real_mode_ip_for_opcode_window() {
        let mut machine = Machine::new(0x20_000);
        machine.cpu.cs.selector = 0x0100;
        machine.cpu.cs.base = 0x1000;
        machine.cpu.rip = 0x1_FFFF;
        machine.mem.write_u8(0x10FFF, 0xA5).unwrap();
        for (addr, byte) in [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70]
            .into_iter()
            .enumerate()
        {
            machine.mem.write_u8(0x1000 + addr as u64, byte).unwrap();
        }

        let context = capture_cpu_failure_context(&machine, 3);

        assert_eq!(context.cs, 0x0100);
        assert_eq!(context.ip, 0xFFFF);
        assert_eq!(context.rip, 0x1_FFFF);
        assert_eq!(context.linear_pc, 0x10FFF);
        assert_eq!(
            context.opcode_bytes,
            [
                Some(0xA5),
                Some(0x10),
                Some(0x20),
                Some(0x30),
                Some(0x40),
                Some(0x50),
                Some(0x60),
                Some(0x70),
            ]
        );
    }

    #[test]
    fn hello_rom_success_output_is_unchanged() {
        let BuiltMachine {
            mut machine, kind, ..
        } = build_machine(&Options::default()).expect("build HELLO");

        let (steps, com1, debug) =
            run_machine(&mut machine, kind, DEFAULT_MAX_STEPS).expect("run HELLO");

        assert!(steps > 0);
        assert!(machine.cpu.halted);
        assert_eq!(com1, EXPECTED_HELLO);
        assert_eq!(debug, EXPECTED_HELLO);
    }

    // -----------------------------------------------------------------
    // VGA / option-ROM diagnostics
    // -----------------------------------------------------------------

    /// A minimal well-formed option ROM: `55 AA`, `blocks` 512-byte blocks,
    /// last byte fixed up so the declared extent sums to zero mod 256.
    fn synthetic_option_rom(blocks: u8) -> Vec<u8> {
        let mut rom = vec![0u8; usize::from(blocks) * OPTION_ROM_BLOCK_BYTES];
        rom[0] = 0x55;
        rom[1] = 0xAA;
        rom[2] = blocks;
        rom[3] = 0xCB; // plausible RETF entry stub; nothing executes it
        let sum = rom.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
        let last = rom.len() - 1;
        rom[last] = rom[last].wrapping_sub(sum);
        rom
    }

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "x86wasm-cli-{tag}-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    struct TempFile(PathBuf);

    impl TempFile {
        fn create(tag: &str, data: &[u8]) -> Self {
            let path = temp_path(tag);
            fs::write(&path, data).expect("write temp file");
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn parse_diagnostic_flags() {
        let parsed = parse_args([
            "--vga-text",
            "--option-rom",
            "vga.bin",
            "--option-rom-base",
            "0xC0000",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                vga_text: true,
                option_rom_path: Some(PathBuf::from("vga.bin")),
                option_rom_base: 0x000C_0000,
                ..Options::default()
            })
        );

        let u = usage();
        assert!(u.contains("--vga-text"), "{u}");
        assert!(u.contains("--option-rom"), "{u}");
    }

    #[test]
    fn option_rom_base_accepts_decimal_and_rejects_garbage() {
        let ParsedArgs::Run(opts) = parse_args(["--option-rom-base", "786432"]).unwrap() else {
            panic!("expected a run");
        };
        assert_eq!(opts.option_rom_base, DEFAULT_OPTION_ROM_BASE);

        assert_eq!(
            parse_args(["--option-rom-base", "0xZZ"]),
            Err(CliError::InvalidAddress("0xZZ".to_string()))
        );
        assert_eq!(
            parse_args(["--option-rom"]),
            Err(CliError::MissingValue("--option-rom"))
        );
    }

    /// Spec: IBM PC option ROM header — `55 AA`, 512-byte block count at
    /// offset 2, declared extent checksums to zero mod 256.
    #[test]
    fn describe_option_rom_accepts_a_well_formed_image() {
        let rom = synthetic_option_rom(4);
        let info = describe_option_rom(DEFAULT_OPTION_ROM_BASE, &rom);

        assert!(info.signature_ok);
        assert_eq!(info.blocks, Some(4));
        assert_eq!(info.declared_bytes, Some(2048));
        assert_eq!(info.checksum, Some(0));
        assert_eq!(info.size, 2048);
        assert!(info.is_valid());
        assert_eq!(
            info.to_string(),
            "option-rom: base=0x000C0000 size=2048 signature=55AA blocks=4 \
             declared=2048 checksum=0x00 status=ok"
        );
    }

    #[test]
    fn describe_option_rom_reports_each_malformed_header() {
        // Bad signature.
        let mut rom = synthetic_option_rom(1);
        rom[0] = 0x00;
        let info = describe_option_rom(0, &rom);
        assert!(!info.signature_ok);
        assert!(!info.is_valid());
        assert!(info.to_string().contains("signature=missing"), "{info}");

        // Bad checksum.
        let mut rom = synthetic_option_rom(1);
        rom[4] = rom[4].wrapping_add(1);
        let info = describe_option_rom(0, &rom);
        assert_eq!(info.checksum, Some(1));
        assert!(!info.is_valid());
        assert!(info.to_string().contains("status=invalid"), "{info}");

        // Size byte claims more than the image holds.
        let mut rom = synthetic_option_rom(1);
        rom[2] = 8;
        let info = describe_option_rom(0, &rom);
        assert_eq!(info.declared_bytes, Some(4096));
        assert_eq!(
            info.checksum, None,
            "cannot sum an extent that does not fit"
        );
        assert!(!info.is_valid());

        // Zero block count.
        let mut rom = synthetic_option_rom(1);
        rom[2] = 0;
        assert!(!describe_option_rom(0, &rom).is_valid());

        // Truncated image.
        let info = describe_option_rom(0, &[0x55, 0xAA]);
        assert!(info.signature_ok);
        assert_eq!(info.blocks, None);
        assert!(!info.is_valid());
    }

    /// The option ROM lands at its base, keeps the firmware ROM windows, and
    /// survives the `reset` that `run_machine` performs.
    #[test]
    fn option_rom_is_mapped_alongside_firmware_and_survives_reset() {
        let rom = synthetic_option_rom(2);
        let file = TempFile::create("optrom", &rom);
        let opts = Options {
            option_rom_path: Some(file.0.clone()),
            ..Options::default()
        };

        let BuiltMachine {
            mut machine,
            kind,
            option_rom,
        } = build_machine(&opts).expect("build with --option-rom");

        assert_eq!(kind, FirmwareKind::Hello);
        assert!(option_rom.expect("info").is_valid());
        assert_eq!(
            machine.mem.read_u8(DEFAULT_OPTION_ROM_BASE).unwrap(),
            0x55,
            "option ROM signature at its base"
        );
        assert_eq!(
            machine.mem.read_u8(DEFAULT_OPTION_ROM_BASE + 1).unwrap(),
            0xAA
        );

        // The HELLO ROM is still mapped, so the run still succeeds.
        let (_, com1, _) = run_machine(&mut machine, kind, DEFAULT_MAX_STEPS).expect("run HELLO");
        assert_eq!(com1, EXPECTED_HELLO);
        assert_eq!(machine.mem.read_u8(DEFAULT_OPTION_ROM_BASE).unwrap(), 0x55);
    }

    #[test]
    fn option_rom_honors_a_custom_base() {
        let rom = synthetic_option_rom(1);
        let file = TempFile::create("optrom-base", &rom);
        let opts = Options {
            option_rom_path: Some(file.0.clone()),
            option_rom_base: 0x000E_0000,
            ..Options::default()
        };

        let built = build_machine(&opts).expect("build");
        assert_eq!(built.option_rom.expect("info").base, 0x000E_0000);
        assert_eq!(built.machine.mem.read_u8(0x000E_0000).unwrap(), 0x55);
    }

    /// A malformed image is still mapped so a developer can inspect it, but it
    /// is reported as invalid.
    #[test]
    fn malformed_option_rom_is_mapped_and_reported_invalid() {
        let file = TempFile::create("optrom-bad", &[0x12, 0x34, 0x56, 0x78]);
        let opts = Options {
            option_rom_path: Some(file.0.clone()),
            ..Options::default()
        };

        let built = build_machine(&opts).expect("build");
        let info = built.option_rom.expect("info");
        assert!(!info.is_valid());
        assert_eq!(
            built.machine.mem.read_u8(DEFAULT_OPTION_ROM_BASE).unwrap(),
            0x12
        );
    }

    #[test]
    fn missing_option_rom_file_is_an_io_error() {
        let opts = Options {
            option_rom_path: Some(temp_path("optrom-absent")),
            ..Options::default()
        };
        assert!(matches!(build_machine(&opts), Err(CliError::Io(_))));
    }

    /// A blank machine dumps 25 empty rows with the mode-03h header fields.
    #[test]
    fn vga_text_dump_reports_a_blank_screen() {
        let built = build_machine(&Options::default()).expect("build HELLO");
        let dump = vga_text_dump(&built.machine);
        let lines: Vec<&str> = dump.lines().collect();

        assert_eq!(lines.len(), 1 + VGA_TEXT_ROWS);
        assert_eq!(
            lines[0],
            "vga-text: cols=80 rows=25 cursor=(0,0) start=0x0000 pitch=80 nonblank_rows=0"
        );
        assert_eq!(lines[1], format!("00 |{}|", " ".repeat(VGA_TEXT_COLS)));
        assert_eq!(
            lines[VGA_TEXT_ROWS],
            format!("24 |{}|", " ".repeat(VGA_TEXT_COLS))
        );
    }

    /// Text written through the guest-facing MMIO entry point shows up in the
    /// dump, and non-printable bytes render as `.`.
    #[test]
    fn vga_text_dump_shows_guest_written_text() {
        let mut built = build_machine(&Options::default()).expect("build HELLO");
        let vga = &mut built.machine.vga;
        for (index, ch) in b"POST OK".iter().enumerate() {
            assert!(vga.mmio_write_u8(0x000B_8000 + (index as u64) * 2, *ch));
            assert!(vga.mmio_write_u8(0x000B_8000 + (index as u64) * 2 + 1, 0x0F));
        }
        // A CP437 box-drawing byte has no ASCII glyph.
        assert!(vga.mmio_write_u8(0x000B_8000 + 14, 0xB0));

        let dump = vga_text_dump(&built.machine);
        let lines: Vec<&str> = dump.lines().collect();

        assert!(lines[0].contains("nonblank_rows=1"), "{}", lines[0]);
        assert!(lines[1].starts_with("00 |POST OK."), "{}", lines[1]);
        assert_eq!(lines[1].len(), "00 ||".len() + VGA_TEXT_COLS);
    }

    /// The dump follows the CRTC Start Address viewport, like `char_at`.
    #[test]
    fn vga_text_dump_follows_the_crtc_start_address() {
        let mut built = build_machine(&Options::default()).expect("build HELLO");
        let vga = &mut built.machine.vga;
        // One row of 80 cells in, write 'Q' so a start address of 80 shows it
        // at row 0 column 0.
        assert!(vga.mmio_write_u8(0x000B_8000 + 80 * 2, b'Q'));
        vga.port_write(0x3D4, 1, 0x0C);
        vga.port_write(0x3D5, 1, 0x00);
        vga.port_write(0x3D4, 1, 0x0D);
        vga.port_write(0x3D5, 1, 80);

        let dump = vga_text_dump(&built.machine);
        let lines: Vec<&str> = dump.lines().collect();
        assert!(lines[0].contains("start=0x0050"), "{}", lines[0]);
        assert!(lines[1].starts_with("00 |Q"), "{}", lines[1]);
    }

    #[test]
    fn usage_mentions_guest_measure_flags() {
        let u = usage();
        assert!(u.contains("--guest-measure"), "{u}");
        assert!(u.contains("--ide-image"), "{u}");
        assert!(u.contains("--floppy-image"), "{u}");
        assert!(u.contains("--cdrom-image"), "{u}");
        assert!(u.contains("--guest-eltorito"), "{u}");
        assert!(u.contains("--guest-freedos-measure"), "{u}");
        assert!(u.contains("--guest-linux-serial-measure"), "{u}");
    }

    #[test]
    fn parse_guest_measure_requires_image() {
        assert_eq!(
            parse_args(["--guest-measure"]),
            Err(CliError::GuestMeasureNeedsImage)
        );
    }

    #[test]
    fn parse_guest_measure_with_ide_image() {
        let parsed = parse_args([
            "--ide-image",
            "disk.bin",
            "--guest-measure",
            "--guest-floppy-first",
            "--steps",
            "100",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                ide_image: Some(PathBuf::from("disk.bin")),
                guest_measure: true,
                guest_floppy_first: true,
                max_steps: 100,
                ..Options::default()
            })
        );
    }

    #[test]
    fn parse_guest_measure_with_cdrom_eltorito() {
        let parsed = parse_args([
            "--cdrom-image",
            "cd.iso",
            "--guest-measure",
            "--guest-eltorito",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                cdrom_image: Some(PathBuf::from("cd.iso")),
                guest_measure: true,
                guest_eltorito: true,
                ..Options::default()
            })
        );
    }

    #[test]
    fn guest_boot_media_policy() {
        let ide_only = Options {
            ide_image: Some(PathBuf::from("a")),
            guest_measure: true,
            ..Options::default()
        };
        assert_eq!(guest_boot_media(&ide_only), GuestBootMedia::IdePrefer);

        let floppy_only = Options {
            floppy_image: Some(PathBuf::from("b")),
            guest_measure: true,
            ..Options::default()
        };
        assert_eq!(guest_boot_media(&floppy_only), GuestBootMedia::FloppyFirst);

        let both_force = Options {
            ide_image: Some(PathBuf::from("a")),
            floppy_image: Some(PathBuf::from("b")),
            guest_floppy_first: true,
            ..Options::default()
        };
        assert_eq!(guest_boot_media(&both_force), GuestBootMedia::FloppyFirst);

        let cdrom_only = Options {
            cdrom_image: Some(PathBuf::from("c")),
            guest_measure: true,
            ..Options::default()
        };
        assert_eq!(guest_boot_media(&cdrom_only), GuestBootMedia::ElTorito);
    }

    /// Synthetic IDE MBR → measure-first halt (not a boot-success claim).
    #[test]
    fn guest_measure_synthetic_ide_hlt() {
        let mut sector = vec![0x90u8; 512];
        sector[0] = 0xF4;
        sector[510] = 0x55;
        sector[511] = 0xAA;
        let file = TempFile::create("guest-mbr", &sector);
        let opts = Options {
            ide_image: Some(file.0.clone()),
            guest_measure: true,
            max_steps: 64,
            ..Options::default()
        };
        let BuiltMachine { mut machine, .. } = build_machine(&opts).expect("build");
        let measure =
            run_guest_measure(&mut machine, guest_boot_media(&opts), opts.max_steps).expect("run");
        let text = measure.to_string();
        assert!(text.contains("guest-measure-v2:"));
        assert!(text.contains("not a boot-success claim"));
        assert!(text.contains("halted"), "{text}");
        assert!(text.contains("checkpoints=["), "{text}");
    }

    #[test]
    fn parse_guest_freedos_measure() {
        let parsed = parse_args(["--guest-freedos-measure", "--steps", "32"]).unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                guest_freedos_measure: true,
                max_steps: 32,
                ..Options::default()
            })
        );
    }

    #[test]
    fn parse_guest_linux_serial_measure() {
        let parsed = parse_args(["--guest-linux-serial-measure"]).unwrap();
        assert_eq!(
            parsed,
            ParsedArgs::Run(Options {
                guest_linux_serial_measure: true,
                ..Options::default()
            })
        );
    }

    #[test]
    fn freedos_measure_synthetic_reports_honesty() {
        let opts = Options {
            guest_freedos_measure: true,
            max_steps: 128,
            ..Options::default()
        };
        let BuiltMachine { mut machine, .. } = build_machine(&opts).expect("build");
        let report = run_freedos_measure(&mut machine, opts.max_steps).expect("run");
        let text = report.to_string();
        assert!(text.contains("freedos-like"));
        assert!(text.contains("NOT an OS boot"));
        assert!(text.contains("does NOT claim a FreeDOS prompt"));
        assert_eq!(report.measure.com1, "FD");
    }

    #[test]
    fn linux_serial_measure_synthetic_reports_gaps() {
        let opts = Options {
            guest_linux_serial_measure: true,
            max_steps: 64,
            ..Options::default()
        };
        let BuiltMachine { mut machine, .. } = build_machine(&opts).expect("build");
        let report = run_linux_serial_measure(&mut machine, opts.max_steps).expect("run");
        let text = report.to_string();
        assert!(text.contains("linux-serial-path"));
        assert!(text.contains("NOT Milestone 2 exit"));
        assert_eq!(report.measure.com1, "LX");
    }
}
