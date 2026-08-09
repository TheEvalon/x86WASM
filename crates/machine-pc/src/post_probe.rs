//! BIOS/SeaBIOS POST first-contact diagnostic harness.
//!
//! [`Machine::probe_post`] single-steps a mapped BIOS image under a bounded
//! instruction budget and records the first thing this machine cannot do, as a
//! structured, assertable report: an unsupported opcode with its eight-byte
//! opcode window, an architectural fault, a memory fault, an unclaimed I/O
//! port, or an unmapped physical page.
//!
//! This is a diagnostic, **not** a claim that POST completes. Nothing here
//! changes architectural behavior; the probe only arms bounded logging in
//! [`crate::ports::PortBus`] while it runs.
//!
//! Spec: Intel SDM Vol. 3 §9.1.4 (reset state, first fetch at `0xFFFFFFF0` with
//! `CS.base = 0xFFFF0000`) and `docs/machine-legacy-glue-r1.md`.

use std::fmt;
use std::path::PathBuf;

use x86_decode::DecodeError;
use x86_interpreter::ExecError;

use crate::ports::{UnclaimedPortAccess, UnmappedMmioAccess};
use crate::step_clock::StepClock;
use crate::{Machine, MachineError};

/// Bytes of instruction stream captured at the failure site.
pub const POST_OPCODE_WINDOW_LEN: usize = 8;

/// Default instruction budget for a POST probe run.
pub const DEFAULT_POST_PROBE_STEPS: u64 = 5_000_000;

/// Environment variable overriding the SeaBIOS image path.
pub const SEABIOS_IMAGE_ENV: &str = "X86WASM_SEABIOS_BIOS";

/// Repo-relative default SeaBIOS image path (git-ignored build output).
pub const SEABIOS_IMAGE_RELATIVE: &str = "firmware/seabios/bios.bin";

/// Locate a SeaBIOS image, or `None` when the tree has not built one.
///
/// Checks [`SEABIOS_IMAGE_ENV`] first, then `firmware/seabios/bios.bin`
/// relative to the repository root. Callers are expected to skip gracefully.
pub fn seabios_image_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(SEABIOS_IMAGE_ENV) {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .to_path_buf();
    let path = repo_root.join(SEABIOS_IMAGE_RELATIVE);
    path.is_file().then_some(path)
}

/// Classification of the first execution failure a probe hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostFailureKind {
    /// Primary opcode absent from the decode tables.
    UnsupportedOpcode(u8),
    /// Instruction fetch ran out of bytes.
    TruncatedInstruction,
    /// Instruction exceeded the SDM 15-byte limit.
    InstructionTooLong,
    /// Decoded, but this encoding/form is not implemented.
    UnsupportedEncoding(u8),
    /// Bus error outside architectural classification.
    MemoryFault(u64),
    /// Fault surfaced to the host instead of being vectored.
    ArchFault { vector: u8, error_code: Option<u16> },
    /// Protected-mode delivery failed (nested `#DF`/triple fault unmodeled).
    ProtectedModeDelivery { vector: u8, reason: String },
    /// Non-execution machine error (ROM/boot-media handling).
    Machine(String),
}

impl fmt::Display for PostFailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOpcode(op) => write!(f, "unsupported opcode 0x{op:02X}"),
            Self::TruncatedInstruction => f.write_str("truncated instruction"),
            Self::InstructionTooLong => f.write_str("instruction too long"),
            Self::UnsupportedEncoding(op) => {
                write!(f, "unsupported encoding for opcode 0x{op:02X}")
            }
            Self::MemoryFault(addr) => write!(f, "memory fault at {addr:#x}"),
            Self::ArchFault { vector, error_code } => match error_code {
                Some(code) => write!(f, "architectural fault vector {vector} error 0x{code:04X}"),
                None => write!(f, "architectural fault vector {vector}"),
            },
            Self::ProtectedModeDelivery { vector, reason } => {
                write!(
                    f,
                    "protected-mode delivery for vector {vector} failed: {reason}"
                )
            }
            Self::Machine(msg) => write!(f, "machine error: {msg}"),
        }
    }
}

/// Legacy prefix bytes that may precede an opcode.
///
/// Spec: Intel SDM Vol. 2 §2.1.1 — group 1 lock/repeat (`F0`, `F2`, `F3`),
/// group 2 segment override and branch hints (`2E`, `36`, `3E`, `26`, `64`,
/// `65`), group 3 operand size (`66`), group 4 address size (`67`).
const LEGACY_PREFIXES: [u8; 11] = [
    0x26, 0x2E, 0x36, 0x3E, 0x64, 0x65, 0x66, 0x67, 0xF0, 0xF2, 0xF3,
];

/// Two-byte opcode escape.
const OPCODE_ESCAPE: u8 = 0x0F;
/// Three-byte opcode escapes following [`OPCODE_ESCAPE`].
const OPCODE_ESCAPE_3BYTE: [u8; 2] = [0x38, 0x3A];

/// The prefixes and the full opcode at a failure site.
///
/// The decoder reports only the byte its tables missed, so a two-byte opcode
/// such as `0F 85` surfaces as `0x85`. Recovering the escape and any prefixes
/// from the captured instruction window keeps the report honest about which
/// encoding is actually missing.
///
/// Spec: Intel SDM Vol. 2 §2.1.1 (instruction format: prefixes, then a one-,
/// two-, or three-byte opcode).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpcodeSite {
    /// Legacy prefixes preceding the opcode, in stream order.
    pub prefixes: Vec<u8>,
    /// Opcode bytes including any `0F` / `0F 38` / `0F 3A` escape.
    pub opcode: Vec<u8>,
}

impl OpcodeSite {
    /// Recover the site from a captured opcode window.
    ///
    /// Returns `None` when the window holds no complete opcode (every byte a
    /// prefix, or an escape with nothing after it).
    pub fn from_window(window: &[Option<u8>; POST_OPCODE_WINDOW_LEN]) -> Option<Self> {
        let mut prefixes = Vec::new();
        let mut index = 0;
        while let Some(Some(byte)) = window.get(index) {
            if !LEGACY_PREFIXES.contains(byte) {
                break;
            }
            prefixes.push(*byte);
            index += 1;
        }

        let first = (*window.get(index)?)?;
        let mut opcode = vec![first];
        if first == OPCODE_ESCAPE {
            index += 1;
            let second = (*window.get(index)?)?;
            opcode.push(second);
            if OPCODE_ESCAPE_3BYTE.contains(&second) {
                index += 1;
                let third = (*window.get(index)?)?;
                opcode.push(third);
            }
        }
        Some(Self { prefixes, opcode })
    }

    /// Whether the decoder's reported byte is this site's final opcode byte.
    ///
    /// A mismatch means the window and the decode error disagree (a wrapped
    /// fetch, say), in which case the raw decoder byte is reported instead.
    pub fn reports(&self, decoded: u8) -> bool {
        self.opcode.last() == Some(&decoded)
    }
}

impl fmt::Display for OpcodeSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.opcode.iter().enumerate() {
            if index != 0 {
                f.write_str(" ")?;
            }
            write!(f, "0x{byte:02X}")?;
        }
        if !self.prefixes.is_empty() {
            f.write_str(" (prefixes")?;
            for byte in &self.prefixes {
                write!(f, " {byte:02X}")?;
            }
            f.write_str(")")?;
        }
        Ok(())
    }
}

/// Where and how the first failure happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostFailure {
    pub kind: PostFailureKind,
    pub cs: u16,
    pub ip: u16,
    pub rip: u64,
    pub linear_pc: u64,
    /// Instruction bytes at the failure site; `None` where the fetch failed.
    pub opcode_bytes: [Option<u8>; POST_OPCODE_WINDOW_LEN],
}

impl PostFailure {
    /// Prefixes and full opcode recovered from the captured window.
    pub fn opcode_site(&self) -> Option<OpcodeSite> {
        OpcodeSite::from_window(&self.opcode_bytes)
    }

    /// The stop-reason text, naming the whole opcode rather than the single
    /// byte the decode tables missed.
    fn kind_text(&self) -> String {
        let site = match self.opcode_site() {
            Some(site) => site,
            None => return self.kind.to_string(),
        };
        match self.kind {
            PostFailureKind::UnsupportedOpcode(op) if site.reports(op) => {
                format!("unsupported opcode {site}")
            }
            PostFailureKind::UnsupportedEncoding(op) if site.reports(op) => {
                format!("unsupported encoding for opcode {site}")
            }
            _ => self.kind.to_string(),
        }
    }
}

impl fmt::Display for PostFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} cs:ip={:04X}:{:04X} rip=0x{:016X} linear_pc=0x{:016X} opcode_bytes=[",
            self.kind_text(),
            self.cs,
            self.ip,
            self.rip,
            self.linear_pc
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

/// Why the probe stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostStopReason {
    /// The CPU executed `HLT` with no pending wake source.
    Halted,
    /// The instruction budget ran out (firmware is still running or spinning).
    StepBudgetExhausted,
    /// Execution failed; see the captured failure.
    Failure(PostFailure),
}

impl fmt::Display for PostStopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Halted => f.write_str("halted"),
            Self::StepBudgetExhausted => f.write_str("step-budget-exhausted"),
            Self::Failure(failure) => write!(f, "{failure}"),
        }
    }
}

/// Structured result of one POST probe run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostReport {
    /// Instructions that retired before the stop.
    pub steps: u64,
    pub stop: PostStopReason,
    /// Ports no device claimed, in first-touch order.
    pub unclaimed_ports: Vec<UnclaimedPortAccess>,
    /// More distinct unclaimed ports existed than the bounded log holds.
    pub unclaimed_port_overflow: bool,
    /// Physical pages outside RAM and every ROM window, in first-touch order.
    pub unmapped_mmio: Vec<UnmappedMmioAccess>,
    pub unmapped_mmio_overflow: bool,
    /// POST checkpoint codes written to port `0x80`, in order.
    pub post_codes: Vec<u8>,
    /// Most recent checkpoint code (survives history overflow).
    pub last_post_code: Option<u8>,
    /// More checkpoint codes were written than the bounded history holds.
    pub post_code_overflow: bool,
    /// Bytes the firmware wrote to COM1.
    pub com1: String,
    /// Bytes the firmware wrote to the `0x402` debug console.
    pub debug: String,
}

impl PostReport {
    /// The captured failure, if the probe stopped on one.
    pub fn failure(&self) -> Option<&PostFailure> {
        match &self.stop {
            PostStopReason::Failure(failure) => Some(failure),
            _ => None,
        }
    }
}

impl fmt::Display for PostReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "post-probe: steps={} stop={}", self.steps, self.stop)?;
        for access in &self.unclaimed_ports {
            writeln!(
                f,
                "  unclaimed-port {} port=0x{:04X} size={} count={} first_value=0x{:08X}",
                if access.write { "out" } else { "in " },
                access.port,
                access.size,
                access.count,
                access.first_value
            )?;
        }
        if self.unclaimed_port_overflow {
            writeln!(f, "  unclaimed-port log overflowed")?;
        }
        for access in &self.unmapped_mmio {
            writeln!(
                f,
                "  unmapped-mmio  {} page=0x{:016X} count={}",
                if access.write { "wr" } else { "rd" },
                access.page,
                access.count
            )?;
        }
        if self.unmapped_mmio_overflow {
            writeln!(f, "  unmapped-mmio log overflowed")?;
        }
        let codes: Vec<String> = self.post_codes.iter().map(|c| format!("{c:02X}")).collect();
        writeln!(
            f,
            "  post-codes=[{}]{} last={}",
            codes.join(" "),
            if self.post_code_overflow {
                " (truncated)"
            } else {
                ""
            },
            match self.last_post_code {
                Some(code) => format!("0x{code:02X}"),
                None => "none".to_string(),
            }
        )?;
        write!(
            f,
            "  com1={:?} debug={:?}",
            self.com1.as_str(),
            self.debug.as_str()
        )
    }
}

impl Machine {
    /// Single-step the mapped firmware under `max_steps` and report first contact.
    ///
    /// Does not reset the machine: map firmware with [`Machine::with_bios_rom`]
    /// (or reset explicitly) first. Diagnostic logging is armed for the duration
    /// of the run only, and the POST checkpoint history is cleared so the report
    /// covers this run alone.
    ///
    /// A time source is armed the same way: when the host has not configured a
    /// [`StepClock`], the probe installs [`StepClock::enabled_default`] for the
    /// run and restores the previous configuration afterwards. Without it,
    /// firmware that waits on the PIT or the RTC exhausts the step budget in a
    /// delay loop and the probe measures nothing. A host-configured clock is
    /// left exactly as it was set.
    pub fn probe_post(&mut self, max_steps: u64) -> PostReport {
        self.ports.clear_diagnostics();
        self.post_diag.reset();
        self.ports.set_probe(true);
        let host_clock = self.step_clock();
        if !host_clock.enabled {
            self.set_step_clock(StepClock::enabled_default());
        }

        let mut steps = 0u64;
        let stop = loop {
            if self.cpu.halted {
                break PostStopReason::Halted;
            }
            if steps >= max_steps {
                break PostStopReason::StepBudgetExhausted;
            }
            match self.step() {
                Ok(()) => steps += 1,
                Err(err) => break PostStopReason::Failure(self.capture_post_failure(&err)),
            }
        };

        self.ports.set_probe(false);
        if !host_clock.enabled {
            self.set_step_clock(host_clock);
        }
        PostReport {
            steps,
            stop,
            unclaimed_ports: self.ports.unclaimed_ports().to_vec(),
            unclaimed_port_overflow: self.ports.unclaimed_port_overflow(),
            unmapped_mmio: self.ports.unmapped_mmio().to_vec(),
            unmapped_mmio_overflow: self.ports.unmapped_mmio_overflow(),
            post_codes: self.post_diag.history().to_vec(),
            last_post_code: self.post_diag.last_code(),
            post_code_overflow: self.post_diag.history_overflow(),
            com1: self.com1_text(),
            debug: self.debug_text(),
        }
    }

    /// Capture the faulting site the same way the interpreter fetches it.
    ///
    /// Spec: Intel SDM Vol. 3 §3.4.2 — a real-mode linear address is the cached
    /// segment base plus the 16-bit offset; instruction fetch wraps IP at 16 bits.
    fn capture_post_failure(&self, err: &MachineError) -> PostFailure {
        let rip = self.cpu.rip;
        let ip = rip as u16;
        let cs_base = self.cpu.cs.base;
        PostFailure {
            kind: classify(err),
            cs: self.cpu.cs.selector,
            ip,
            rip,
            linear_pc: cs_base.wrapping_add(u64::from(ip)),
            opcode_bytes: std::array::from_fn(|index| {
                let offset = ip.wrapping_add(index as u16);
                self.mem
                    .read_u8(cs_base.wrapping_add(u64::from(offset)))
                    .ok()
            }),
        }
    }
}

fn classify(err: &MachineError) -> PostFailureKind {
    match err {
        MachineError::Exec(ExecError::Decode(DecodeError::UnsupportedOpcode(op))) => {
            PostFailureKind::UnsupportedOpcode(*op)
        }
        MachineError::Exec(ExecError::Decode(DecodeError::Truncated)) => {
            PostFailureKind::TruncatedInstruction
        }
        MachineError::Exec(ExecError::Decode(DecodeError::TooLong)) => {
            PostFailureKind::InstructionTooLong
        }
        MachineError::Exec(ExecError::Unsupported(op)) => PostFailureKind::UnsupportedEncoding(*op),
        MachineError::Exec(ExecError::MemoryFault(addr)) => PostFailureKind::MemoryFault(*addr),
        MachineError::Exec(ExecError::ArchFault { vector, error_code }) => {
            PostFailureKind::ArchFault {
                vector: *vector,
                error_code: *error_code,
            }
        }
        MachineError::Exec(ExecError::ProtectedModeExceptionDelivery { vector, reason }) => {
            PostFailureKind::ProtectedModeDelivery {
                vector: *vector,
                reason: reason.to_string(),
            }
        }
        other => PostFailureKind::Machine(other.to_string()),
    }
}
