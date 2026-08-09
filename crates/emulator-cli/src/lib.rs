//! Native CLI helpers for the x86WASM emulator runner.

use machine_pc::{build_hello_rom, Machine, MachineError, EXPECTED_HELLO};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Default instruction budget when `--steps` is omitted.
pub const DEFAULT_MAX_STEPS: u64 = 100_000;

const OPCODE_WINDOW_LEN: usize = 8;

/// Parsed CLI options (firmware path + run budget).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Lab / HELLO path — high map only via [`Machine::load_rom`].
    pub rom_path: Option<PathBuf>,
    /// Classic BIOS path — dual-map via [`Machine::load_bios_rom`].
    pub bios_path: Option<PathBuf>,
    pub max_steps: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rom_path: None,
            bios_path: None,
            max_steps: DEFAULT_MAX_STEPS,
        }
    }
}

/// Result of parsing argv (excluding program name).
#[derive(Clone, Debug, PartialEq, Eq)]
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
    RomAndBios,
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
            Self::RomAndBios => write!(f, "Use only one of --rom or --bios"),
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
        "Usage: emulator-cli [--rom path.bin | --bios path.bin] [--steps N]\n\
         --rom   Load a lab ROM at top-of-4GiB only (HELLO-style).\n\
         --bios  Load a legacy BIOS via dual map (top-of-4GiB + below-1MiB alias).\n\
         Default ROM prints '{EXPECTED_HELLO}' via COM1 and port 0x402."
    )
}

/// Parse CLI arguments (excluding argv[0]).
pub fn parse_args<I, S>(args: I) -> Result<ParsedArgs, CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut opts = Options::default();
    let mut iter = args.into_iter();
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
    Ok(ParsedArgs::Run(opts))
}

/// Build a machine and install firmware from `opts`.
///
/// `--bios` uses [`Machine::with_bios_rom`] (→ [`Machine::load_bios_rom`]).
/// `--rom` / default HELLO use [`Machine::load_rom`].
pub fn build_machine(opts: &Options) -> Result<(Machine, FirmwareKind), CliError> {
    const RAM: usize = 16 * 1024 * 1024;
    if let Some(path) = &opts.bios_path {
        let data = read_file(path)?;
        let machine = Machine::with_bios_rom(RAM, &data).map_err(machine_err)?;
        return Ok((machine, FirmwareKind::Bios));
    }
    let mut machine = Machine::new(RAM);
    if let Some(path) = &opts.rom_path {
        let data = read_file(path)?;
        machine.load_rom(&data).map_err(machine_err)?;
        Ok((machine, FirmwareKind::Rom))
    } else {
        machine.load_rom(&build_hello_rom()).map_err(machine_err)?;
        Ok((machine, FirmwareKind::Hello))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
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
                rom_path: None,
                max_steps: 42,
            })
        );
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
        let (mut machine, kind) = build_machine(&opts).expect("build with --bios");
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
        let (mut machine, kind) = build_machine(&Options::default()).expect("build HELLO");

        let (steps, com1, debug) =
            run_machine(&mut machine, kind, DEFAULT_MAX_STEPS).expect("run HELLO");

        assert!(steps > 0);
        assert!(machine.cpu.halted);
        assert_eq!(com1, EXPECTED_HELLO);
        assert_eq!(debug, EXPECTED_HELLO);
    }
}
