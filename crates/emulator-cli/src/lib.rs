//! Native CLI helpers for the x86WASM emulator runner.

use machine_pc::{build_hello_rom, Machine, MachineError, EXPECTED_HELLO};
use std::fs;
use std::path::{Path, PathBuf};

/// Default instruction budget when `--steps` is omitted.
pub const DEFAULT_MAX_STEPS: u64 = 100_000;

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

/// CLI argument / usage errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliError {
    UnknownArgument(String),
    MissingValue(&'static str),
    InvalidSteps(String),
    RomAndBios,
    Io(String),
    Machine(String),
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
        }
    }
}

impl std::error::Error for CliError {}

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

/// Run until HLT / step budget; validate HELLO output when applicable.
pub fn run_machine(
    machine: &mut Machine,
    kind: FirmwareKind,
    max_steps: u64,
) -> Result<(u64, String, String), CliError> {
    machine.reset();
    let steps = machine
        .run(max_steps)
        .map_err(|e| CliError::Machine(format!("Execution error: {e}")))?;
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
}
