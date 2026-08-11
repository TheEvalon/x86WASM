//! Guest disk boot measure harness v2/v4/v8 (FreeDOS/Linux serial-path prep).
//!
//! Loads a boot sector / El Torito / synthetic FreeDOS-like image, then reuses
//! [`Machine::probe_post`] to record the **first** stop reason plus serial
//! capture, optional VGA summary, named checkpoints, and a structured
//! first-failure class (decode/#UD, device, INT13 CF, hang location). This
//! does **not** claim FreeDOS prompt, Linux userspace, or Milestone 2 boot
//! success.
//!
//! Spec: IBM PC BIOS INT 19h handoff + El Torito 1.0 no-emul load + existing
//! POST probe diagnostics (`docs/boot-r8-guest-measure-v2.md`,
//! `docs/boot-r9-freedos-measure.md`, `docs/boot-r10-freedos-first-failure.md`,
//! `docs/boot-r10-linux-serial-first-failure.md`,
//! `docs/boot-r11-freedos-bda-equipment.md`,
//! `docs/boot-r11-linux-boot-protocol-inspect.md`,
//! `docs/boot-r12-freedos-next-gap.md`,
//! `docs/boot-r12-linux-bzimage-early.md`,
//! `docs/boot-r13-freedos-with-media.md`,
//! `docs/boot-r13-eltorito-media-classify.md`,
//! `docs/boot-r13-linux-setup-deeper.md`,
//! `docs/boot-r14-freedos-next-gap.md`,
//! `docs/boot-r14-linux-eltorito-measure.md`,
//! `docs/boot-r14-mbr-vbr-chain.md`,
//! `docs/boot-r14-post-with-media.md`,
//! `docs/boot-r15-freedos-next.md`,
//! `docs/boot-r15-linux-next.md`).

use crate::boot_media::{classify_machine_int19_media, Int19BootMediaClass};
use crate::fat12::locate_freedos_kernel_on_machine;
use crate::post_probe::{PostFailureKind, PostReport, PostStopReason};
use crate::{Machine, MachineError};

/// Harness schema version for CLI/report consumers (v2 checkpoints + serial).
pub const GUEST_BOOT_MEASURE_VERSION: u32 = 2;

/// FreeDOS-like / Linux-serial measure report schema (v8 = FAT12 kernel-name next-gap).
pub const GUEST_OS_MEASURE_VERSION: u32 = 8;

/// BDA equipment list word (`0040:0010`). Spec: RBIL memory map / IBM BIOS.
pub const BDA_EQUIPMENT: u64 = 0x410;
/// BDA number of hard disk drives (`0040:0075`). Spec: RBIL memory map.
pub const BDA_HD_COUNT: u64 = 0x475;

/// Linux real-mode boot-protocol magic at offset `0x202` (`Documentation/x86/boot.rst`).
pub const LINUX_BOOT_HEADER_MAGIC: [u8; 4] = *b"HdrS";
/// Classic boot-sector signature at offset `0x1FE`.
pub const LINUX_BOOT_FLAG_AA55: u16 = 0xAA55;
/// Minimum buffer length to reach `loadflags` (`0x211`).
pub const LINUX_BOOT_HEADER_MIN_LEN: usize = 0x212;

/// Which host boot helper to use before measuring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestBootMedia {
    /// [`Machine::load_mbr_to_7c00`] — IDE LBA0 prefer, else floppy.
    IdePrefer,
    /// [`Machine::load_floppy_boot_to_7c00`] — floppy CHS `(0,0,1)` only.
    FloppyFirst,
    /// [`Machine::load_eltorito_to_7c00`] — no-emulation CD boot image.
    ElTorito,
    /// [`Machine::load_active_vbr_to_7c00`] — active partition VBR → `0x7C00`.
    ActiveVbr,
    /// [`Machine::load_bzimage_realmode_setup`] + arm entry at `+0x200` (R15).
    BzImageSetup,
}

/// Host handoff used for FreeDOS next-gap classify (MBR vs VBR chain).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FreedosHandoff {
    /// LBA0 / MBR sector at `0x7C00` (R13 media-attached class).
    #[default]
    MbrSector,
    /// Active-partition VBR at `0x7C00` (R14 chain past MBR-only).
    ActiveVbr,
}

/// Ordered progress markers recorded during a v2 measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestBootCheckpoint {
    /// Selected media helper completed without setup error.
    MediaLoaded,
    /// Guest `CS:IP` points at the handoff entry.
    CsIpArmed,
    /// POST probe began executing guest instructions.
    ProbeStarted,
    /// At least one COM1 or debug-console byte was observed at stop.
    SerialObserved,
    /// At least one printable VGA text cell observed at stop.
    VgaObserved,
    /// Probe recorded a terminal stop reason.
    StopRecorded,
}

/// Result of preparing media + measuring first failure (v2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestBootMeasure {
    /// Schema version ([`GUEST_BOOT_MEASURE_VERSION`]).
    pub version: u32,
    /// How the boot image was selected.
    pub media: GuestBootMedia,
    /// Checkpoints reached before/during the probe.
    pub checkpoints: Vec<GuestBootCheckpoint>,
    /// COM1 bytes captured at stop (FreeDOS/Linux serial-path signal).
    pub com1: String,
    /// Port `0x402` debug console bytes at stop.
    pub debug: String,
    /// Compact VGA text summary (non-blank rows only); empty if blank.
    pub vga_summary: String,
    /// Probe report (first failure / halt / budget). **Not** a success claim.
    pub report: PostReport,
}

impl GuestBootMeasure {
    /// True when any serial/debug byte was captured (does not imply boot success).
    pub fn serial_captured(&self) -> bool {
        !self.com1.is_empty() || !self.debug.is_empty()
    }

    /// True when VGA text had any printable non-space glyph.
    pub fn vga_captured(&self) -> bool {
        !self.vga_summary.is_empty()
    }

    /// Human-readable stop class for FreeDOS/Linux bring-up triage.
    pub fn stop_class(&self) -> &'static str {
        match &self.report.stop {
            PostStopReason::Halted => "halted",
            PostStopReason::StepBudgetExhausted => "step-budget",
            PostStopReason::Failure(_) => "first-failure",
        }
    }
}

impl std::fmt::Display for GuestBootMeasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "guest-measure-v{}: media={} stop-class={} (measure-first; not a boot-success claim)",
            self.version,
            match self.media {
                GuestBootMedia::IdePrefer => "ide-prefer",
                GuestBootMedia::FloppyFirst => "floppy-first",
                GuestBootMedia::ElTorito => "eltorito",
                GuestBootMedia::ActiveVbr => "active-vbr",
                GuestBootMedia::BzImageSetup => "bzimage-setup",
            },
            self.stop_class()
        )?;
        write!(f, "  checkpoints=[")?;
        for (i, cp) in self.checkpoints.iter().enumerate() {
            if i != 0 {
                f.write_str(", ")?;
            }
            f.write_str(match cp {
                GuestBootCheckpoint::MediaLoaded => "media-loaded",
                GuestBootCheckpoint::CsIpArmed => "cs-ip-armed",
                GuestBootCheckpoint::ProbeStarted => "probe-started",
                GuestBootCheckpoint::SerialObserved => "serial-observed",
                GuestBootCheckpoint::VgaObserved => "vga-observed",
                GuestBootCheckpoint::StopRecorded => "stop-recorded",
            })?;
        }
        writeln!(f, "]")?;
        writeln!(
            f,
            "  serial: com1={:?} debug={:?}",
            self.com1.as_str(),
            self.debug.as_str()
        )?;
        if !self.vga_summary.is_empty() {
            writeln!(f, "  vga: {}", self.vga_summary)?;
        }
        write!(f, "{}", self.report)
    }
}

/// Which synthetic / path-specific OS measure was requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestOsMeasureKind {
    /// In-tree FreeDOS-*like* MBR+payload fixture (not FreeDOS).
    FreeDosLike,
    /// Serial-path stub toward 32-bit Linux console (not a kernel boot).
    LinuxSerialPath,
}

/// Structured first-stop / first-failure class for guest OS-path triage.
///
/// Derived from [`GuestBootMeasure::report`]; never claims OS boot success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestFirstFailureClass {
    /// Intentional synthetic `HLT` after banner (fixture complete; not an OS).
    SyntheticHalt,
    /// Instruction budget exhausted before halt/failure.
    StepBudget,
    /// Decode tables lack the primary opcode.
    UnsupportedOpcode { opcode: u8 },
    /// Encoding/form unimplemented.
    UnsupportedEncoding { opcode: u8 },
    /// Truncated fetch.
    TruncatedInstruction,
    /// SDM 15-byte limit exceeded.
    InstructionTooLong,
    /// Memory / bus fault.
    MemoryFault { addr: u64 },
    /// Architectural exception surfaced to the host.
    ArchFault { vector: u8 },
    /// Protected-mode delivery failed.
    ProtectedModeDelivery { vector: u8 },
    /// Machine-level setup/runtime error recorded by the probe.
    MachineError,
    /// Probe stopped with unclaimed I/O as the only actionable signal.
    UnclaimedIo { port: u16 },
    /// Probe stopped with unmapped MMIO as the only actionable signal.
    UnmappedMmio { page: u64 },
    /// Host INT 13h probe returned CF set (disk service failure).
    Int13Cf { ah: u8 },
}

/// Next actionable gap after a FreeDOS-like measure (beyond the first-failure tag).
///
/// Used when the fixture already reached `synthetic-halt` (or to point back at
/// `first_failure`). Does **not** claim a FreeDOS prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FreedosNextGap {
    /// Host INT 13h AH=41h (or follow-on) returned CF — disk service gap.
    HostInt13Cf { ah: u8 },
    /// BDA equipment / HD-count fields disagree with attached media.
    BdaDiskMismatch,
    /// IVT vector `0x13` is null — guest cannot reach host INT 13h without SeaBIOS.
    GuestInt13IvtMissing,
    /// INT 19h-candidate media attached — past no-media reboot-loop class; still
    /// need SeaBIOS guest disk path / real FreeDOS (not a prompt). MBR-only handoff.
    MediaAttachedBeyondRebootLoop,
    /// Active-partition VBR executed (host chain) and halted — past media-attached
    /// class; stub has no COMMAND.COM / FreeDOS kernel.
    ExecutedVbrMissingCommand,
    /// FAT12 root lists `KERNEL.SYS` / `COMMAND.COM` — past VBR-missing-command;
    /// next gap is host/guest load of that file (still not a FreeDOS prompt).
    KernelNameLocatedMissingLoad,
    /// Fixture halted cleanly without INT19-candidate media; next need is real image + POST.
    RealImageAndFirmware,
    /// Non-halt first failure already names the gap — see `first_failure`.
    SeeFirstFailure,
}

impl FreedosNextGap {
    /// Stable triage tag (ASCII kebab-case).
    pub fn tag(&self) -> &'static str {
        match self {
            Self::HostInt13Cf { .. } => "host-int13-cf",
            Self::BdaDiskMismatch => "bda-disk-mismatch",
            Self::GuestInt13IvtMissing => "guest-int13-ivt-missing",
            Self::MediaAttachedBeyondRebootLoop => "media-attached-beyond-reboot-loop",
            Self::ExecutedVbrMissingCommand => "executed-vbr-missing-command",
            Self::KernelNameLocatedMissingLoad => "kernel-name-located-missing-load",
            Self::RealImageAndFirmware => "real-image-and-firmware",
            Self::SeeFirstFailure => "see-first-failure",
        }
    }

    /// Host-note sentence for reports (static).
    pub fn host_note(&self) -> &'static str {
        match self {
            Self::HostInt13Cf { .. } => {
                "Next gap: host INT 13h probe CF — disk/extensions service failure"
            }
            Self::BdaDiskMismatch => {
                "Next gap: BDA 0040:0010/0075 mismatch vs attached media"
            }
            Self::GuestInt13IvtMissing => {
                "Next gap: IVT INT 13h null — need SeaBIOS (or install host stub); not FreeDOS prompt"
            }
            Self::MediaAttachedBeyondRebootLoop => {
                "Next gap: INT19-candidate media attached (past no-media reboot loop); still need SeaBIOS guest INT13 + real FreeDOS — NOT a prompt"
            }
            Self::ExecutedVbrMissingCommand => {
                "Next gap: VBR executed (host MBR→VBR chain) then synthetic halt — missing COMMAND.COM / FreeDOS kernel; NOT a FreeDOS prompt"
            }
            Self::KernelNameLocatedMissingLoad => {
                "Next gap: FAT12 root has KERNEL.SYS/COMMAND.COM name — missing cluster load/exec via INT13; NOT a FreeDOS prompt"
            }
            Self::RealImageAndFirmware => {
                "Next gap: real FreeDOS image + SeaBIOS POST (fixture halt is not progress)"
            }
            Self::SeeFirstFailure => "Next gap: see first-failure class (non-halt stop)",
        }
    }
}

impl std::fmt::Display for FreedosNextGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.tag())
    }
}

/// Read IVT far pointer for interrupt `vector` (offset then segment, little-endian).
fn read_ivt_far(machine: &Machine, vector: u8) -> (u16, u16) {
    let base = u64::from(vector) * 4;
    let off = u16::from(machine.mem.read_u8(base).unwrap_or(0))
        | (u16::from(machine.mem.read_u8(base + 1).unwrap_or(0)) << 8);
    let seg = u16::from(machine.mem.read_u8(base + 2).unwrap_or(0))
        | (u16::from(machine.mem.read_u8(base + 3).unwrap_or(0)) << 8);
    (off, seg)
}

/// Classify the **next** FreeDOS-path gap after a measure (INT13 / BDA / IVT / media).
///
/// Spec: RBIL IVT + BDA disk fields; IBM INT 13h / INT 19h. Does not claim FreeDOS boot.
pub fn classify_freedos_next_gap(
    machine: &Machine,
    first_failure: &GuestFirstFailureClass,
    int13_probe: &Int13ProbeSnapshot,
) -> FreedosNextGap {
    classify_freedos_next_gap_with_handoff(
        machine,
        first_failure,
        int13_probe,
        FreedosHandoff::MbrSector,
    )
}

/// Like [`classify_freedos_next_gap`], but distinguishes MBR-only vs VBR-chain handoff.
///
/// Spec: OSDev Boot Sequence MBR→VBR; still not FreeDOS / COMMAND.COM.
pub fn classify_freedos_next_gap_with_handoff(
    machine: &Machine,
    first_failure: &GuestFirstFailureClass,
    int13_probe: &Int13ProbeSnapshot,
    handoff: FreedosHandoff,
) -> FreedosNextGap {
    if !matches!(first_failure, GuestFirstFailureClass::SyntheticHalt) {
        return FreedosNextGap::SeeFirstFailure;
    }
    if int13_probe.failed() {
        return FreedosNextGap::HostInt13Cf { ah: int13_probe.ah };
    }
    let expect_hd = u8::from(machine.ide.present && !machine.ide.image.is_empty());
    let bda_hd = machine.mem.read_u8(BDA_HD_COUNT).unwrap_or(0xFF);
    let bda_equip = machine.mem.read_u8(BDA_EQUIPMENT).unwrap_or(0xFF);
    if bda_hd != expect_hd || bda_equip != machine.equipment_byte() {
        return FreedosNextGap::BdaDiskMismatch;
    }
    let (off, seg) = read_ivt_far(machine, 0x13);
    if off == 0 && seg == 0 {
        return FreedosNextGap::GuestInt13IvtMissing;
    }
    if classify_machine_int19_media(machine).is_int19_candidate() {
        return match handoff {
            FreedosHandoff::ActiveVbr => {
                // R15: FAT12 root name locate advances past executed-vbr-missing-command.
                if locate_freedos_kernel_on_machine(machine).name_found() {
                    FreedosNextGap::KernelNameLocatedMissingLoad
                } else {
                    FreedosNextGap::ExecutedVbrMissingCommand
                }
            }
            FreedosHandoff::MbrSector => FreedosNextGap::MediaAttachedBeyondRebootLoop,
        };
    }
    FreedosNextGap::RealImageAndFirmware
}

/// Host view of whether attached media would leave the POST no-media reboot loop.
///
/// Derived from [`classify_machine_int19_media`]. Does **not** claim FreeDOS/Linux boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaBootReadiness {
    /// No usable sector attached — SeaBIOS INT 19h → `boot_fail` / CF9 reboot loop.
    NoMedia,
    /// Media present but not an INT 19h candidate (bad signature / no active part).
    AttachedNotCandidate,
    /// INT 19h-candidate HD or floppy attached (past no-media reboot-loop class).
    Int19Candidate,
}

impl MediaBootReadiness {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::NoMedia => "no-media",
            Self::AttachedNotCandidate => "attached-not-candidate",
            Self::Int19Candidate => "int19-candidate",
        }
    }

    pub fn from_int19_class(class: Int19BootMediaClass) -> Self {
        match class {
            Int19BootMediaClass::TooShort => Self::NoMedia,
            Int19BootMediaClass::MissingSignature | Int19BootMediaClass::HdSignatureOnly => {
                Self::AttachedNotCandidate
            }
            Int19BootMediaClass::HdActivePartition { .. }
            | Int19BootMediaClass::FloppyBootSector => Self::Int19Candidate,
        }
    }
}

impl std::fmt::Display for MediaBootReadiness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.tag())
    }
}

impl GuestFirstFailureClass {
    /// Stable triage tag for CLI/docs (ASCII kebab-case).
    pub fn tag(&self) -> &'static str {
        match self {
            Self::SyntheticHalt => "synthetic-halt",
            Self::StepBudget => "step-budget",
            Self::UnsupportedOpcode { .. } => "unsupported-opcode",
            Self::UnsupportedEncoding { .. } => "unsupported-encoding",
            Self::TruncatedInstruction => "truncated-instruction",
            Self::InstructionTooLong => "instruction-too-long",
            Self::MemoryFault { .. } => "memory-fault",
            Self::ArchFault { .. } => "arch-fault",
            Self::ProtectedModeDelivery { .. } => "pm-delivery",
            Self::MachineError => "machine-error",
            Self::UnclaimedIo { .. } => "unclaimed-io",
            Self::UnmappedMmio { .. } => "unmapped-mmio",
            Self::Int13Cf { .. } => "int13-cf",
        }
    }

    /// Coarse triage bucket matching R10 acceptance (decode/#UD, device, INT13 CF, hang).
    pub fn bucket(&self) -> &'static str {
        match self {
            Self::UnsupportedOpcode { .. }
            | Self::UnsupportedEncoding { .. }
            | Self::TruncatedInstruction
            | Self::InstructionTooLong => "decode-ud",
            Self::ArchFault { vector } if *vector == 6 => "decode-ud",
            Self::UnclaimedIo { .. } | Self::UnmappedMmio { .. } => "device",
            Self::Int13Cf { .. } => "int13-cf",
            Self::StepBudget => "hang",
            Self::SyntheticHalt => "halted",
            Self::ArchFault { .. }
            | Self::ProtectedModeDelivery { .. }
            | Self::MemoryFault { .. }
            | Self::MachineError => "other",
        }
    }

    /// Gap string contributed by this class (appended to static path gaps).
    pub fn gap_note(&self) -> &'static str {
        match self {
            Self::SyntheticHalt => {
                "Synthetic fixture halted after banner — not FreeDOS/Linux progress"
            }
            Self::StepBudget => {
                "Step budget exhausted before a terminal halt/failure (hang location in report)"
            }
            Self::UnsupportedOpcode { .. } => "First failure: unsupported opcode (CPU decode gap)",
            Self::UnsupportedEncoding { .. } => {
                "First failure: unsupported encoding (CPU form gap)"
            }
            Self::TruncatedInstruction => "First failure: truncated instruction fetch",
            Self::InstructionTooLong => "First failure: instruction too long",
            Self::MemoryFault { .. } => "First failure: memory/bus fault",
            Self::ArchFault { .. } => "First failure: architectural fault (exception delivery)",
            Self::ProtectedModeDelivery { .. } => {
                "First failure: protected-mode exception delivery"
            }
            Self::MachineError => "First failure: machine/runtime error in probe",
            Self::UnclaimedIo { .. } => "First actionable signal: unclaimed I/O port (device)",
            Self::UnmappedMmio { .. } => "First actionable signal: unmapped MMIO page (device)",
            Self::Int13Cf { .. } => "Host INT 13h probe returned CF (disk service failure)",
        }
    }
}

impl std::fmt::Display for GuestFirstFailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SyntheticHalt => f.write_str("synthetic-halt"),
            Self::StepBudget => f.write_str("step-budget"),
            Self::UnsupportedOpcode { opcode } => {
                write!(f, "unsupported-opcode:0x{opcode:02X}")
            }
            Self::UnsupportedEncoding { opcode } => {
                write!(f, "unsupported-encoding:0x{opcode:02X}")
            }
            Self::TruncatedInstruction => f.write_str("truncated-instruction"),
            Self::InstructionTooLong => f.write_str("instruction-too-long"),
            Self::MemoryFault { addr } => write!(f, "memory-fault:{addr:#x}"),
            Self::ArchFault { vector } => write!(f, "arch-fault:vec={vector}"),
            Self::ProtectedModeDelivery { vector } => {
                write!(f, "pm-delivery:vec={vector}")
            }
            Self::MachineError => f.write_str("machine-error"),
            Self::UnclaimedIo { port } => write!(f, "unclaimed-io:0x{port:04X}"),
            Self::UnmappedMmio { page } => write!(f, "unmapped-mmio:{page:#x}"),
            Self::Int13Cf { ah } => write!(f, "int13-cf:AH={ah:02X}"),
        }
    }
}

/// Classify the first stop / first failure from a v2 guest measure report.
///
/// When `int13_probe` reports CF on an otherwise soft stop (halt/budget),
/// surface [`GuestFirstFailureClass::Int13Cf`]. Hard decode/device failures win.
pub fn classify_guest_first_failure(
    report: &PostReport,
    int13_probe: Option<&Int13ProbeSnapshot>,
) -> GuestFirstFailureClass {
    let class = match &report.stop {
        PostStopReason::Halted => GuestFirstFailureClass::SyntheticHalt,
        PostStopReason::StepBudgetExhausted => {
            if let Some(access) = report.unclaimed_ports.first() {
                GuestFirstFailureClass::UnclaimedIo { port: access.port }
            } else if let Some(access) = report.unmapped_mmio.first() {
                GuestFirstFailureClass::UnmappedMmio { page: access.page }
            } else {
                GuestFirstFailureClass::StepBudget
            }
        }
        PostStopReason::Failure(failure) => match &failure.kind {
            PostFailureKind::UnsupportedOpcode(op) => {
                GuestFirstFailureClass::UnsupportedOpcode { opcode: *op }
            }
            PostFailureKind::UnsupportedEncoding(op) => {
                GuestFirstFailureClass::UnsupportedEncoding { opcode: *op }
            }
            PostFailureKind::TruncatedInstruction => GuestFirstFailureClass::TruncatedInstruction,
            PostFailureKind::InstructionTooLong => GuestFirstFailureClass::InstructionTooLong,
            PostFailureKind::MemoryFault(addr) => {
                GuestFirstFailureClass::MemoryFault { addr: *addr }
            }
            PostFailureKind::ArchFault { vector, .. } => {
                GuestFirstFailureClass::ArchFault { vector: *vector }
            }
            PostFailureKind::ProtectedModeDelivery { vector, .. } => {
                GuestFirstFailureClass::ProtectedModeDelivery { vector: *vector }
            }
            PostFailureKind::Machine(_) => GuestFirstFailureClass::MachineError,
        },
    };
    if matches!(
        class,
        GuestFirstFailureClass::SyntheticHalt | GuestFirstFailureClass::StepBudget
    ) {
        if let Some(probe) = int13_probe {
            if probe.cf {
                return GuestFirstFailureClass::Int13Cf { ah: probe.ah };
            }
        }
    }
    class
}

/// Guest stop location (`CS:EIP`) for hang / first-failure triage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestFailureSite {
    /// `CS` selector at stop.
    pub cs: u16,
    /// `EIP` at stop.
    pub eip: u32,
}

impl std::fmt::Display for GuestFailureSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04X}:{:08X}", self.cs, self.eip)
    }
}

/// Snapshot of a host INT 13h register probe (AH/CF/DL).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Int13ProbeSnapshot {
    pub dl: u8,
    pub ah: u8,
    pub cf: bool,
}

impl Int13ProbeSnapshot {
    /// True when the host INT 13h probe returned CF set.
    pub fn failed(&self) -> bool {
        self.cf
    }
}

/// v7 OS-path measure: wraps v2 plus honesty / first-failure / next-gap / media readiness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestOsMeasure {
    /// Schema version ([`GUEST_OS_MEASURE_VERSION`]).
    pub version: u32,
    /// Path kind.
    pub kind: GuestOsMeasureKind,
    /// Underlying v2 measure (serial/VGA/checkpoints/stop).
    pub measure: GuestBootMeasure,
    /// Structured first-stop / first-failure class.
    pub first_failure: GuestFirstFailureClass,
    /// Coarse bucket (`decode-ud` / `device` / `int13-cf` / `hang` / …).
    pub failure_bucket: &'static str,
    /// Hang / stop location (`CS:EIP`).
    pub failure_site: GuestFailureSite,
    /// Host INT 13h AH=41h probe after the guest stop.
    pub int13_probe: Int13ProbeSnapshot,
    /// FreeDOS-path next-gap class (INT13 / BDA / IVT / media). Always set.
    pub next_gap: FreedosNextGap,
    /// Whether attached media is an INT 19h candidate (past no-media reboot loop).
    pub media_readiness: MediaBootReadiness,
    /// Explicit non-claim sentence for reports/CLI.
    pub honesty: &'static str,
    /// Remaining gaps toward a real guest (not M2 exit), including class note.
    pub gaps: Vec<&'static str>,
    /// Optional host-image / BDA notes when the fixture already halted cleanly.
    pub host_notes: Vec<&'static str>,
}

impl std::fmt::Display for GuestOsMeasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            GuestOsMeasureKind::FreeDosLike => "freedos-like",
            GuestOsMeasureKind::LinuxSerialPath => "linux-serial-path",
        };
        writeln!(
            f,
            "guest-os-measure-v{}: kind={} first-failure={} bucket={} site={} next-gap={} media={} (NOT an OS boot / NOT Milestone 2 exit)",
            self.version,
            kind,
            self.first_failure,
            self.failure_bucket,
            self.failure_site,
            self.next_gap,
            self.media_readiness
        )?;
        writeln!(f, "  honesty: {}", self.honesty)?;
        writeln!(
            f,
            "  int13-probe: DL={:02X}h AH={:02X}h CF={}",
            self.int13_probe.dl,
            self.int13_probe.ah,
            u8::from(self.int13_probe.cf)
        )?;
        write!(f, "  gaps=[")?;
        for (i, g) in self.gaps.iter().enumerate() {
            if i != 0 {
                f.write_str("; ")?;
            }
            f.write_str(g)?;
        }
        writeln!(f, "]")?;
        if !self.host_notes.is_empty() {
            write!(f, "  host-notes=[")?;
            for (i, n) in self.host_notes.iter().enumerate() {
                if i != 0 {
                    f.write_str("; ")?;
                }
                f.write_str(n)?;
            }
            writeln!(f, "]")?;
        }
        write!(f, "{}", self.measure)
    }
}

/// Error from [`inspect_linux_boot_protocol_header`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinuxBootProtocolError {
    /// Buffer shorter than [`LINUX_BOOT_HEADER_MIN_LEN`].
    Truncated,
    /// Offset `0x1FE` is not `0xAA55`.
    BadBootFlag,
    /// Offset `0x202` is not `HdrS`.
    BadMagic,
}

impl std::fmt::Display for LinuxBootProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => f.write_str("linux-boot-protocol: truncated"),
            Self::BadBootFlag => f.write_str("linux-boot-protocol: bad boot_flag"),
            Self::BadMagic => f.write_str("linux-boot-protocol: bad HdrS magic"),
        }
    }
}

/// Minimal real-mode Linux boot-protocol header fields (inspect-only).
///
/// Spec: Linux `Documentation/x86/boot.rst` (boot protocol ≥ 2.00). This does
/// **not** load a bzImage, enter protected mode, or claim a kernel boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinuxBootProtocolHeader {
    /// Offset `0x1F1` — sectors of setup code (0 means 4).
    pub setup_sects: u8,
    /// Offset `0x1F2` — `root_flags` (readonly root).
    pub root_flags: u16,
    /// Offset `0x1F4` — `syssize` (16-byte paragraphs; protocol-dependent).
    pub syssize: u32,
    /// Offset `0x1FE` — must be [`LINUX_BOOT_FLAG_AA55`].
    pub boot_flag: u16,
    /// Offset `0x202` — must be [`LINUX_BOOT_HEADER_MAGIC`].
    pub header_magic: [u8; 4],
    /// Offset `0x206` — boot protocol version (`0x0200` = 2.00).
    pub version: u16,
    /// Offset `0x211` — `loadflags` (bit0 LOADED_HIGH, etc.).
    pub loadflags: u8,
    /// Offset `0x214` — 32-bit kernel entry (`code32_start`), if buffer long enough.
    pub code32_start: Option<u32>,
}

impl LinuxBootProtocolHeader {
    /// True when `loadflags` bit 0 (`LOADED_HIGH`) is set.
    pub fn loaded_high(&self) -> bool {
        self.loadflags & 0x01 != 0
    }
}

/// Inspect a Linux real-mode boot-protocol header in `buf` (bzImage-shaped).
///
/// Does **not** vendor or execute a kernel. Spec: Linux boot protocol
/// (`Documentation/x86/boot.rst`) — fields at fixed offsets from image start.
pub fn inspect_linux_boot_protocol_header(
    buf: &[u8],
) -> Result<LinuxBootProtocolHeader, LinuxBootProtocolError> {
    if buf.len() < LINUX_BOOT_HEADER_MIN_LEN {
        return Err(LinuxBootProtocolError::Truncated);
    }
    let boot_flag = u16::from_le_bytes([buf[0x1FE], buf[0x1FF]]);
    if boot_flag != LINUX_BOOT_FLAG_AA55 {
        return Err(LinuxBootProtocolError::BadBootFlag);
    }
    let header_magic = [buf[0x202], buf[0x203], buf[0x204], buf[0x205]];
    if header_magic != LINUX_BOOT_HEADER_MAGIC {
        return Err(LinuxBootProtocolError::BadMagic);
    }
    let setup_sects = buf[0x1F1];
    let root_flags = u16::from_le_bytes([buf[0x1F2], buf[0x1F3]]);
    let syssize = u32::from_le_bytes([buf[0x1F4], buf[0x1F5], buf[0x1F6], buf[0x1F7]]);
    let version = u16::from_le_bytes([buf[0x206], buf[0x207]]);
    let loadflags = buf[0x211];
    let code32_start = if buf.len() >= 0x218 {
        Some(u32::from_le_bytes([
            buf[0x214], buf[0x215], buf[0x216], buf[0x217],
        ]))
    } else {
        None
    };
    Ok(LinuxBootProtocolHeader {
        setup_sects,
        root_flags,
        syssize,
        boot_flag,
        header_magic,
        version,
        loadflags,
        code32_start,
    })
}

/// Build a minimal synthetic Linux boot-protocol header buffer for harnesses.
///
/// Not a bzImage — only the inspectable header fields used by tests/docs.
pub fn synthetic_linux_boot_protocol_header(
    setup_sects: u8,
    version: u16,
    loadflags: u8,
    code32_start: u32,
) -> Vec<u8> {
    let mut buf = vec![0u8; 0x220];
    buf[0x1F1] = setup_sects;
    buf[0x1FE] = (LINUX_BOOT_FLAG_AA55 & 0xFF) as u8;
    buf[0x1FF] = (LINUX_BOOT_FLAG_AA55 >> 8) as u8;
    buf[0x202..0x206].copy_from_slice(&LINUX_BOOT_HEADER_MAGIC);
    buf[0x206] = (version & 0xFF) as u8;
    buf[0x207] = (version >> 8) as u8;
    buf[0x211] = loadflags;
    buf[0x214..0x218].copy_from_slice(&code32_start.to_le_bytes());
    buf
}

/// Effective setup sector count per Linux boot protocol (`0` means 4).
pub fn linux_setup_sect_count(setup_sects: u8) -> u8 {
    if setup_sects == 0 {
        4
    } else {
        setup_sects
    }
}

/// Byte length of the real-mode blob: boot sector + setup sectors.
pub fn linux_realmode_bytes(setup_sects: u8) -> usize {
    (usize::from(linux_setup_sect_count(setup_sects)) + 1) * 512
}

/// Classic real-mode load base for the Linux setup blob (`0x90000`).
///
/// Spec: Linux `Documentation/x86/boot.rst` — real-mode kernel at `0x90000`.
pub const LINUX_REALMODE_LOAD_ADDR: u64 = 0x9_0000;

/// Next step after a successful early bzImage header/setup classify.
///
/// Does **not** claim a serial shell or full boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BzImageNextStep {
    /// Protocol too old for the fields we inspect (`version` < 2.00).
    UnsupportedOldProtocol { version: u16 },
    /// Protocol ≥ 2.02 but `cmd_line_ptr` is zero — need cmdline setup before run.
    NeedCmdlinePtr,
    /// `LOADED_HIGH` and protocol ≥ 2.10 with `init_size == 0` — need init_size.
    NeedInitSize,
    /// Real-mode setup is loadable; next is execute setup (out of scope here).
    RunRealModeSetup,
    /// `LOADED_HIGH` set — next is load protected kernel / jump `code32_start`.
    LoadHighProtectedKernel { code32_start: u32 },
}

impl BzImageNextStep {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::UnsupportedOldProtocol { .. } => "unsupported-old-protocol",
            Self::NeedCmdlinePtr => "need-cmdline-ptr",
            Self::NeedInitSize => "need-init-size",
            Self::RunRealModeSetup => "run-real-mode-setup",
            Self::LoadHighProtectedKernel { .. } => "load-high-protected-kernel",
        }
    }
}

impl std::fmt::Display for BzImageNextStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedOldProtocol { version } => {
                write!(f, "unsupported-old-protocol:{version:#06x}")
            }
            Self::LoadHighProtectedKernel { code32_start } => {
                write!(f, "load-high-protected-kernel:{code32_start:#010x}")
            }
            other => f.write_str(other.tag()),
        }
    }
}

/// Early bzImage triage result (inspect + setup-size check; no kernel execution).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BzImageEarlyClass {
    /// Header parse failed.
    BadHeader(LinuxBootProtocolError),
    /// Buffer shorter than real-mode setup blob implied by `setup_sects`.
    IncompleteSetup {
        setup_sects: u8,
        have: usize,
        need: usize,
    },
    /// Header + setup size OK — reports the next failure mode toward boot.
    SetupLoadable {
        setup_sects: u8,
        version: u16,
        loaded_high: bool,
        code32_start: Option<u32>,
        next: BzImageNextStep,
    },
}

impl BzImageEarlyClass {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::BadHeader(_) => "bad-header",
            Self::IncompleteSetup { .. } => "incomplete-setup",
            Self::SetupLoadable { .. } => "setup-loadable",
        }
    }
}

impl std::fmt::Display for BzImageEarlyClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadHeader(e) => write!(f, "bad-header:{e}"),
            Self::IncompleteSetup {
                setup_sects,
                have,
                need,
            } => write!(
                f,
                "incomplete-setup:sects={setup_sects} have={have} need={need}"
            ),
            Self::SetupLoadable { next, .. } => write!(f, "setup-loadable:next={next}"),
        }
    }
}

/// Classify a host bzImage (or synthetic header) far enough to name the next gap.
///
/// Loads nothing and executes nothing. Spec: Linux `Documentation/x86/boot.rst`.
pub fn classify_bzimage_early(buf: &[u8]) -> BzImageEarlyClass {
    let hdr = match inspect_linux_boot_protocol_header(buf) {
        Ok(h) => h,
        Err(e) => return BzImageEarlyClass::BadHeader(e),
    };
    let need = linux_realmode_bytes(hdr.setup_sects);
    if buf.len() < need {
        return BzImageEarlyClass::IncompleteSetup {
            setup_sects: hdr.setup_sects,
            have: buf.len(),
            need,
        };
    }
    let next = if hdr.version < 0x0200 {
        BzImageNextStep::UnsupportedOldProtocol {
            version: hdr.version,
        }
    } else if hdr.loaded_high() {
        BzImageNextStep::LoadHighProtectedKernel {
            code32_start: hdr.code32_start.unwrap_or(0x0010_0000),
        }
    } else {
        BzImageNextStep::RunRealModeSetup
    };
    BzImageEarlyClass::SetupLoadable {
        setup_sects: hdr.setup_sects,
        version: hdr.version,
        loaded_high: hdr.loaded_high(),
        code32_start: hdr.code32_start,
        next,
    }
}

/// Deepen [`classify_bzimage_early`] with protocol 2.02+ cmdline / 2.10+ init_size.
///
/// Spec: Linux `Documentation/x86/boot.rst` — `cmd_line_ptr` @ `0x228` (2.02+),
/// `init_size` @ `0x260` (2.10+). Does not execute setup or claim a shell.
pub fn classify_bzimage_setup_deeper(buf: &[u8]) -> BzImageEarlyClass {
    let mut class = classify_bzimage_early(buf);
    let BzImageEarlyClass::SetupLoadable {
        setup_sects,
        version,
        loaded_high,
        code32_start,
        next,
    } = &class
    else {
        return class;
    };
    let setup_sects = *setup_sects;
    let version = *version;
    let loaded_high = *loaded_high;
    let code32_start = *code32_start;
    let next = *next;

    let refined = match next {
        BzImageNextStep::UnsupportedOldProtocol { .. } => next,
        BzImageNextStep::RunRealModeSetup if version >= 0x0202 => {
            let cmd = if buf.len() >= 0x22C {
                u32::from_le_bytes([buf[0x228], buf[0x229], buf[0x22A], buf[0x22B]])
            } else {
                0
            };
            if cmd == 0 {
                BzImageNextStep::NeedCmdlinePtr
            } else {
                next
            }
        }
        BzImageNextStep::LoadHighProtectedKernel { code32_start: c32 }
            if version >= 0x020A && loaded_high =>
        {
            let init_size = if buf.len() >= 0x264 {
                u32::from_le_bytes([buf[0x260], buf[0x261], buf[0x262], buf[0x263]])
            } else {
                0
            };
            if init_size == 0 {
                BzImageNextStep::NeedInitSize
            } else {
                BzImageNextStep::LoadHighProtectedKernel { code32_start: c32 }
            }
        }
        other => other,
    };
    class = BzImageEarlyClass::SetupLoadable {
        setup_sects,
        version,
        loaded_high,
        code32_start,
        next: refined,
    };
    class
}

/// El Torito / ATAPI media boot readiness (host classify; not SeaBIOS CD stack).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElToritoMediaBootClass {
    /// No ATAPI CD-ROM medium attached.
    NoMedium,
    /// Catalog / ISO parse failed.
    CatalogError(String),
    /// Default entry not bootable (`88h` missing).
    NotBootable,
    /// Floppy/HDD emulation media type (unsupported here).
    UnsupportedEmulation { media_type: u8 },
    /// No-emulation bootable image — INT 19h CD candidate fields.
    NoEmulCandidate {
        load_rba: u32,
        sector_count: u16,
        load_segment: u16,
    },
}

impl ElToritoMediaBootClass {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::NoMedium => "no-medium",
            Self::CatalogError(_) => "catalog-error",
            Self::NotBootable => "not-bootable",
            Self::UnsupportedEmulation { .. } => "unsupported-emulation",
            Self::NoEmulCandidate { .. } => "no-emul-candidate",
        }
    }

    /// True when a no-emul boot image is present (past empty-CD / no-media class).
    pub fn is_boot_candidate(&self) -> bool {
        matches!(self, Self::NoEmulCandidate { .. })
    }
}

impl std::fmt::Display for ElToritoMediaBootClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CatalogError(e) => write!(f, "catalog-error:{e}"),
            Self::UnsupportedEmulation { media_type } => {
                write!(f, "unsupported-emulation:{media_type:#04x}")
            }
            Self::NoEmulCandidate {
                load_rba,
                sector_count,
                load_segment,
            } => write!(
                f,
                "no-emul-candidate:rba={load_rba} sectors={sector_count} seg={load_segment:#06x}"
            ),
            other => f.write_str(other.tag()),
        }
    }
}

/// Classify attached ATAPI El Torito media for boot candidacy.
///
/// Spec: El Torito 1.0 — Validation + Default Entry. Does not load/execute the
/// boot image (see [`Machine::load_eltorito_to_7c00`]).
pub fn classify_eltorito_media_boot(machine: &Machine) -> ElToritoMediaBootClass {
    if !machine.ide.is_atapi_cdrom() || machine.ide.atapi_medium_image().is_none() {
        return ElToritoMediaBootClass::NoMedium;
    }
    match machine.inspect_atapi_el_torito() {
        Err(e) => ElToritoMediaBootClass::CatalogError(e.to_string()),
        Ok(info) if !info.bootable => ElToritoMediaBootClass::NotBootable,
        Ok(info) if info.media_type != firmware_interface::EL_TORITO_MEDIA_NO_EMUL => {
            ElToritoMediaBootClass::UnsupportedEmulation {
                media_type: info.media_type,
            }
        }
        Ok(info) => ElToritoMediaBootClass::NoEmulCandidate {
            load_rba: info.load_rba,
            sector_count: info.sector_count,
            load_segment: info.effective_load_segment(),
        },
    }
}

/// El Torito catalog → boot-image payload classify (R15 deepen).
///
/// After a no-emul candidate is present, peek at the boot image bytes for a
/// bzImage-shaped header vs a synthetic HLT stub. Spec: El Torito 1.0 + Linux
/// boot.rst. Does **not** claim a serial shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElToritoPayloadClass {
    /// Catalog / medium classify (not a loaded no-emul peek).
    Media(ElToritoMediaBootClass),
    /// No-emul boot image starts with HLT / non-HdrS stub.
    NoEmulHltStub {
        load_rba: u32,
        sector_count: u16,
        load_segment: u16,
    },
    /// No-emul boot image parses as a Linux setup/bzImage header.
    NoEmulBzImage {
        load_rba: u32,
        sector_count: u16,
        load_segment: u16,
        bzimage: BzImageEarlyClass,
    },
}

impl ElToritoPayloadClass {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Media(_) => "media",
            Self::NoEmulHltStub { .. } => "no-emul-hlt-stub",
            Self::NoEmulBzImage { .. } => "no-emul-bzimage",
        }
    }

    pub fn is_bzimage_candidate(&self) -> bool {
        matches!(
            self,
            Self::NoEmulBzImage {
                bzimage: BzImageEarlyClass::SetupLoadable { .. },
                ..
            }
        )
    }
}

impl std::fmt::Display for ElToritoPayloadClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Media(m) => write!(f, "media:{m}"),
            Self::NoEmulHltStub {
                load_rba,
                sector_count,
                load_segment,
            } => write!(
                f,
                "no-emul-hlt-stub:rba={load_rba} sectors={sector_count} seg={load_segment:#06x}"
            ),
            Self::NoEmulBzImage {
                load_rba,
                sector_count,
                load_segment,
                bzimage,
            } => write!(
                f,
                "no-emul-bzimage:rba={load_rba} sectors={sector_count} seg={load_segment:#06x} bz={bzimage}"
            ),
        }
    }
}

/// Classify El Torito boot catalog candidacy **and** peek at the boot image payload.
///
/// Spec: El Torito 1.0 (`sector_count` × 512 load length from `load_rba` × 2048);
/// Linux boot.rst header inspect. Host-only; not SeaBIOS CD INT 13h.
pub fn classify_eltorito_boot_payload(machine: &Machine) -> ElToritoPayloadClass {
    let media = classify_eltorito_media_boot(machine);
    let ElToritoMediaBootClass::NoEmulCandidate {
        load_rba,
        sector_count,
        load_segment,
    } = media
    else {
        return ElToritoPayloadClass::Media(media);
    };
    let Some(image) = machine.ide.atapi_medium_image() else {
        return ElToritoPayloadClass::Media(ElToritoMediaBootClass::NoMedium);
    };
    let Some(byte_len) = (sector_count as usize).checked_mul(512).filter(|n| *n > 0) else {
        return ElToritoPayloadClass::NoEmulHltStub {
            load_rba,
            sector_count,
            load_segment,
        };
    };
    let Some(src_off) = (load_rba as usize).checked_mul(firmware_interface::EL_TORITO_SECTOR_BYTES)
    else {
        return ElToritoPayloadClass::Media(ElToritoMediaBootClass::CatalogError(
            "load_rba overflow".into(),
        ));
    };
    let Some(src_end) = src_off.checked_add(byte_len) else {
        return ElToritoPayloadClass::Media(ElToritoMediaBootClass::CatalogError(
            "boot image OOB".into(),
        ));
    };
    if src_end > image.len() {
        return ElToritoPayloadClass::Media(ElToritoMediaBootClass::CatalogError(
            "boot image truncated".into(),
        ));
    }
    let boot = &image[src_off..src_end];
    let bz = classify_bzimage_setup_deeper(boot);
    if matches!(bz, BzImageEarlyClass::SetupLoadable { .. }) {
        return ElToritoPayloadClass::NoEmulBzImage {
            load_rba,
            sector_count,
            load_segment,
            bzimage: bz,
        };
    }
    ElToritoPayloadClass::NoEmulHltStub {
        load_rba,
        sector_count,
        load_segment,
    }
}

/// Combined Linux / El Torito media boot readiness (host classify + optional measure).
///
/// Spec: El Torito 1.0 + Linux `Documentation/x86/boot.rst`. Does **not** claim
/// a Linux serial shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinuxMediaBootClass {
    /// No CD and no bzImage buffer supplied.
    NoMedia,
    /// Attached CD classify (may or may not be a boot candidate).
    ElTorito(ElToritoMediaBootClass),
    /// Host bzImage buffer classify (setup deepen).
    BzImage(BzImageEarlyClass),
    /// El Torito candidate **and** a separate bzImage was also classified.
    ElToritoPlusBzImage {
        eltorito: ElToritoMediaBootClass,
        bzimage: BzImageEarlyClass,
    },
}

impl LinuxMediaBootClass {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::NoMedia => "no-media",
            Self::ElTorito(_) => "eltorito",
            Self::BzImage(_) => "bzimage",
            Self::ElToritoPlusBzImage { .. } => "eltorito-plus-bzimage",
        }
    }

    /// True when El Torito is a no-emul candidate and/or bzImage setup is loadable.
    pub fn is_boot_candidate(&self) -> bool {
        match self {
            Self::NoMedia => false,
            Self::ElTorito(e) => e.is_boot_candidate(),
            Self::BzImage(b) => matches!(b, BzImageEarlyClass::SetupLoadable { .. }),
            Self::ElToritoPlusBzImage { eltorito, bzimage } => {
                eltorito.is_boot_candidate()
                    || matches!(bzimage, BzImageEarlyClass::SetupLoadable { .. })
            }
        }
    }
}

impl std::fmt::Display for LinuxMediaBootClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMedia => f.write_str("no-media"),
            Self::ElTorito(e) => write!(f, "eltorito:{e}"),
            Self::BzImage(b) => write!(f, "bzimage:{b}"),
            Self::ElToritoPlusBzImage { eltorito, bzimage } => {
                write!(f, "eltorito-plus-bzimage:cd={eltorito} bz={bzimage}")
            }
        }
    }
}

/// Classify Linux-path media: optional attached El Torito CD and/or bzImage bytes.
///
/// Prefer CD classify when ATAPI medium is present; fold in `bzimage` when given.
/// Spec: El Torito 1.0; Linux boot.rst. Not a serial-shell claim.
pub fn classify_linux_media_boot(machine: &Machine, bzimage: Option<&[u8]>) -> LinuxMediaBootClass {
    let cd = if machine.ide.is_atapi_cdrom() && machine.ide.atapi_medium_image().is_some() {
        Some(classify_eltorito_media_boot(machine))
    } else {
        None
    };
    let bz = bzimage.map(classify_bzimage_setup_deeper);
    match (cd, bz) {
        (None, None) => LinuxMediaBootClass::NoMedia,
        (Some(e), None) => LinuxMediaBootClass::ElTorito(e),
        (None, Some(b)) => LinuxMediaBootClass::BzImage(b),
        (Some(e), Some(b)) => LinuxMediaBootClass::ElToritoPlusBzImage {
            eltorito: e,
            bzimage: b,
        },
    }
}

/// Next actionable gap on the Linux serial/media path (R15 deepen).
///
/// Spec: Linux `Documentation/x86/boot.rst` real-mode entry at setup+`0x200`.
/// Does **not** claim a serial shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinuxNextGap {
    /// Non-halt first failure already names the gap.
    SeeFirstFailure,
    /// El Torito / serial stub halted; next is real bzImage setup.
    SyntheticMediaHalt,
    /// Real-mode setup loaded at `0x90000` but `CS:IP` not armed at entry.
    SetupLoadedMissingEntry,
    /// Setup entry executed (synthetic HLT) — next is protected-mode / high kernel.
    SetupExecutedMissingProtectedKernel,
    /// Need a real bzImage + firmware path beyond fixtures.
    RealKernelAndFirmware,
}

impl LinuxNextGap {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::SeeFirstFailure => "see-first-failure",
            Self::SyntheticMediaHalt => "synthetic-media-halt",
            Self::SetupLoadedMissingEntry => "setup-loaded-missing-entry",
            Self::SetupExecutedMissingProtectedKernel => "setup-executed-missing-protected-kernel",
            Self::RealKernelAndFirmware => "real-kernel-and-firmware",
        }
    }

    pub fn host_note(&self) -> &'static str {
        match self {
            Self::SeeFirstFailure => "Next gap: see first-failure class (non-halt stop)",
            Self::SyntheticMediaHalt => {
                "Next gap: synthetic El Torito/serial halt — need bzImage setup entry; NOT a serial shell"
            }
            Self::SetupLoadedMissingEntry => {
                "Next gap: real-mode setup loaded but CS:IP not at entry 0x200; NOT a serial shell"
            }
            Self::SetupExecutedMissingProtectedKernel => {
                "Next gap: real-mode setup entry executed (synthetic) — missing protected kernel/jump; NOT a serial shell"
            }
            Self::RealKernelAndFirmware => {
                "Next gap: real bzImage + SeaBIOS/firmware path (fixture is not progress)"
            }
        }
    }
}

impl std::fmt::Display for LinuxNextGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.tag())
    }
}

/// Classify Linux-path next-gap after a measure / load helper (R15).
///
/// `setup_entry_armed` is true when [`Machine::arm_bzimage_realmode_entry`] ran.
/// Spec: boot.rst entry at offset `0x200`. Not a shell claim.
pub fn classify_linux_next_gap(
    first_failure: &GuestFirstFailureClass,
    media: &LinuxMediaBootClass,
    setup_entry_armed: bool,
) -> LinuxNextGap {
    if !matches!(first_failure, GuestFirstFailureClass::SyntheticHalt) {
        return LinuxNextGap::SeeFirstFailure;
    }
    if setup_entry_armed {
        return LinuxNextGap::SetupExecutedMissingProtectedKernel;
    }
    if matches!(
        media,
        LinuxMediaBootClass::BzImage(BzImageEarlyClass::SetupLoadable { .. })
            | LinuxMediaBootClass::ElToritoPlusBzImage {
                bzimage: BzImageEarlyClass::SetupLoadable { .. },
                ..
            }
    ) {
        return LinuxNextGap::SetupLoadedMissingEntry;
    }
    if media.is_boot_candidate() {
        return LinuxNextGap::SyntheticMediaHalt;
    }
    LinuxNextGap::RealKernelAndFirmware
}

/// Synthetic bzImage real-mode blob: header + setup entry at `+0x200` prints `LX` then HLT.
///
/// Spec: Linux boot.rst — entry at offset `0x200` for protocol ≥ 2.00. Real
/// kernels place a short jump at `0x200` so `HdrS` at `0x202` stays intact.
/// Not a kernel; fixture for [`Machine::measure_linux_bzimage_setup_entry`].
pub fn synthetic_linux_bzimage_setup_hlt() -> Vec<u8> {
    let need = linux_realmode_bytes(1);
    let mut buf = synthetic_linux_boot_protocol_header(1, 0x0200, 0, 0);
    buf.resize(need, 0x90);
    // Entry at +0x200: jmp short to 0x220 (keeps HdrS @ 0x202).
    // After the 2-byte jmp, IP=0x202; rel=+0x1E → 0x220.
    buf[0x200] = 0xEB;
    buf[0x201] = 0x1E;
    let code: &[u8] = &[
        0xBA, 0xF8, 0x03, // mov dx, 0x03F8
        0xB0, b'L', //
        0xEE, //
        0xB0, b'X', //
        0xEE, //
        0xF4, // hlt
    ];
    buf[0x220..0x220 + code.len()].copy_from_slice(code);
    buf
}

/// Error from [`Machine::load_bzimage_realmode_setup`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BzImageLoadError {
    Classify(LinuxBootProtocolError),
    IncompleteSetup { have: usize, need: usize },
    RamTooSmall,
}

impl std::fmt::Display for BzImageLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Classify(e) => write!(f, "bzImage load: {e}"),
            Self::IncompleteSetup { have, need } => {
                write!(f, "bzImage load: incomplete setup have={have} need={need}")
            }
            Self::RamTooSmall => f.write_str("bzImage load: RAM too small"),
        }
    }
}

impl From<LinuxBootProtocolError> for BzImageLoadError {
    fn from(value: LinuxBootProtocolError) -> Self {
        Self::Classify(value)
    }
}

impl Machine {
    /// Seed classic BDA diskette / equipment / HD-count fields from attached media.
    ///
    /// Writes:
    /// - [`BDA_EQUIPMENT`] (`0040:0010`) low byte = [`Self::equipment_byte`]
    /// - [`BDA_HD_COUNT`] (`0040:0075`) = 1 when IDE image present, else 0
    ///
    /// Spec: RBIL BIOS Data Area — equipment list + HD count. Host helper only;
    /// does not claim SeaBIOS POST filled the BDA.
    pub fn seed_bda_disk_equipment(&mut self) -> Result<(), MachineError> {
        let equip = self.equipment_byte();
        self.mem
            .write_u8(BDA_EQUIPMENT, equip)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.mem
            .write_u8(BDA_EQUIPMENT + 1, 0)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        let hd = u8::from(self.ide.present && !self.ide.image.is_empty());
        self.mem
            .write_u8(BDA_HD_COUNT, hd)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        Ok(())
    }

    /// Load boot image per `media`, then [`Self::probe_post`] under `max_steps`.
    ///
    /// Returns [`MachineError`] only for media/signature/RAM problems before
    /// execution starts. Execution stops are always captured inside the report.
    ///
    /// v2 also records checkpoints and copies COM1/debug serial at the stop;
    /// R9 adds a compact VGA text summary when printable glyphs exist.
    pub fn measure_guest_boot(
        &mut self,
        media: GuestBootMedia,
        max_steps: u64,
    ) -> Result<GuestBootMeasure, MachineError> {
        match media {
            GuestBootMedia::IdePrefer => self.load_mbr_to_7c00()?,
            GuestBootMedia::FloppyFirst => self.load_floppy_boot_to_7c00()?,
            GuestBootMedia::ElTorito => self.load_eltorito_to_7c00()?,
            GuestBootMedia::ActiveVbr => {
                self.load_active_vbr_to_7c00()?;
            }
            GuestBootMedia::BzImageSetup => {
                return Err(MachineError::NoBootMedia);
            }
        }
        let mut checkpoints = vec![
            GuestBootCheckpoint::MediaLoaded,
            GuestBootCheckpoint::CsIpArmed,
            GuestBootCheckpoint::ProbeStarted,
        ];
        let report = self.probe_post(max_steps);
        let com1 = report.com1.clone();
        let debug = report.debug.clone();
        if !com1.is_empty() || !debug.is_empty() {
            checkpoints.push(GuestBootCheckpoint::SerialObserved);
        }
        let vga_summary = vga_text_summary(self);
        if !vga_summary.is_empty() {
            checkpoints.push(GuestBootCheckpoint::VgaObserved);
        }
        checkpoints.push(GuestBootCheckpoint::StopRecorded);
        Ok(GuestBootMeasure {
            version: GUEST_BOOT_MEASURE_VERSION,
            media,
            checkpoints,
            com1,
            debug,
            vga_summary,
            report,
        })
    }

    /// Host INT 13h AH=41h extensions probe on `DL=80h` (status / CF snapshot).
    pub fn probe_int13_hd_extensions_status(&mut self) -> Int13ProbeSnapshot {
        use crate::int13::{INT13_AH_CHECK_EXTENSIONS, INT13_DRIVE_HD0, INT13_EXT_MAGIC_IN};
        use x86_core::CpuState;
        self.cpu.set_ah(INT13_AH_CHECK_EXTENSIONS);
        self.cpu.set_gpr_u16(CpuState::RBX, INT13_EXT_MAGIC_IN);
        self.cpu.set_gpr_u8_low(CpuState::RDX, INT13_DRIVE_HD0);
        self.service_int13();
        Int13ProbeSnapshot {
            dl: INT13_DRIVE_HD0,
            ah: self.cpu.ah(),
            cf: self.cpu.rflags & 1 != 0,
        }
    }

    fn finish_os_measure(
        &mut self,
        kind: GuestOsMeasureKind,
        measure: GuestBootMeasure,
        honesty: &'static str,
        mut gaps: Vec<&'static str>,
        mut host_notes: Vec<&'static str>,
        handoff: FreedosHandoff,
    ) -> GuestOsMeasure {
        let int13_probe = self.probe_int13_hd_extensions_status();
        let first_failure = classify_guest_first_failure(&measure.report, Some(&int13_probe));
        let failure_bucket = first_failure.bucket();
        let failure_site = GuestFailureSite {
            cs: measure.report.stop_site.cs,
            eip: measure.report.stop_site.eip,
        };
        gaps.push(first_failure.gap_note());
        let media_readiness =
            MediaBootReadiness::from_int19_class(classify_machine_int19_media(self));
        let next_gap = if kind == GuestOsMeasureKind::FreeDosLike {
            classify_freedos_next_gap_with_handoff(self, &first_failure, &int13_probe, handoff)
        } else if matches!(first_failure, GuestFirstFailureClass::SyntheticHalt) {
            if media_readiness == MediaBootReadiness::Int19Candidate {
                FreedosNextGap::MediaAttachedBeyondRebootLoop
            } else {
                FreedosNextGap::RealImageAndFirmware
            }
        } else {
            FreedosNextGap::SeeFirstFailure
        };
        if matches!(first_failure, GuestFirstFailureClass::SyntheticHalt) {
            host_notes.push(
                "Synthetic-halt reached — next gap is real guest image / firmware POST, not fixture polish",
            );
            if kind == GuestOsMeasureKind::FreeDosLike {
                host_notes.push(
                    "BDA 0040:0010/0075 seeded from host media; still not SeaBIOS equipment init",
                );
                host_notes.push(next_gap.host_note());
            }
            if kind == GuestOsMeasureKind::LinuxSerialPath {
                host_notes.push(
                    "Use classify_bzimage_early / classify_bzimage_setup_deeper / classify_linux_media_boot / load_bzimage_realmode_setup on a host bzImage; no kernel exec here",
                );
            }
        } else if kind == GuestOsMeasureKind::FreeDosLike {
            host_notes.push(next_gap.host_note());
        }
        GuestOsMeasure {
            version: GUEST_OS_MEASURE_VERSION,
            kind,
            measure,
            first_failure,
            failure_bucket,
            failure_site,
            int13_probe,
            next_gap,
            media_readiness,
            honesty,
            gaps,
            host_notes,
        }
    }

    /// Attach the in-tree FreeDOS-*like* IDE fixture (if no IDE yet) and measure.
    ///
    /// The fixture is a signed MBR that prints `FD` to COM1 and a VGA glyph,
    /// then `HLT`, plus a second-sector payload marker. It is **not** FreeDOS
    /// and must never be reported as a FreeDOS prompt.
    ///
    /// Before probing, seeds BDA equipment / HD count from attached media
    /// ([`Self::seed_bda_disk_equipment`]).
    pub fn measure_freedos_like(&mut self, max_steps: u64) -> Result<GuestOsMeasure, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_ide_image(synthetic_freedos_like_disk());
        }
        self.seed_bda_disk_equipment()?;
        let measure = self.measure_guest_boot(GuestBootMedia::IdePrefer, max_steps)?;
        Ok(self.finish_os_measure(
            GuestOsMeasureKind::FreeDosLike,
            measure,
            "Synthetic FreeDOS-like MBR+payload only — does NOT claim a FreeDOS prompt or kernel.",
            vec![
                "No FreeDOS image vendored; fixture is synthetic",
                "Guest INT 13h still needs SeaBIOS (host subset is not an IVT body)",
                "Incomplete firmware POST / devices / opcodes for real FreeDOS",
                "No claim of COMMAND.COM or FreeDOS prompt",
            ],
            vec![],
            FreedosHandoff::MbrSector,
        ))
    }

    /// Attach INT 19h-bootable FreeDOS stub HD (if needed) and measure with media classify.
    ///
    /// Uses [`crate::boot_media::synthetic_int19_freedos_stub_hd`] so the attached
    /// image is an INT 19h candidate (past the POST no-media reboot-loop class).
    /// Host [`Self::load_mbr_to_7c00`] still executes the **MBR** (HLT), not the
    /// partition VBR — report classifies media readiness + next-gap honestly.
    ///
    /// Does **not** claim a FreeDOS prompt.
    pub fn measure_freedos_with_bootable_media(
        &mut self,
        max_steps: u64,
    ) -> Result<GuestOsMeasure, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_freedos_stub_hd_for_int19();
        }
        self.seed_bda_disk_equipment()?;
        let measure = self.measure_guest_boot(GuestBootMedia::IdePrefer, max_steps)?;
        Ok(self.finish_os_measure(
            GuestOsMeasureKind::FreeDosLike,
            measure,
            "INT19-candidate FreeDOS stub HD measured — does NOT claim a FreeDOS prompt or SeaBIOS INT19 success.",
            vec![
                "Synthetic INT19-candidate media only (active partition + stub VBR)",
                "Host MBR handoff executes MBR HLT, not partition VBR / FreeDOS kernel",
                "Guest INT 13h still needs SeaBIOS (host subset is not an IVT body)",
                "Past no-media reboot-loop class only when media_readiness=int19-candidate",
                "No claim of COMMAND.COM or FreeDOS prompt",
            ],
            vec![
                "Media readiness distinguishes no-media reboot loop from attached-candidate path",
            ],
            FreedosHandoff::MbrSector,
        ))
    }

    /// Attach FreeDOS stub HD and measure via **active-partition VBR** handoff.
    ///
    /// Past [`FreedosNextGap::MediaAttachedBeyondRebootLoop`]: host
    /// [`Self::load_active_vbr_to_7c00`] runs the stub VBR (`FD` + HLT). With an
    /// INT 13h IVT stub installed, next-gap is
    /// [`FreedosNextGap::ExecutedVbrMissingCommand`].
    ///
    /// Still **not** a FreeDOS prompt / COMMAND.COM.
    pub fn measure_freedos_vbr_chain(
        &mut self,
        max_steps: u64,
    ) -> Result<GuestOsMeasure, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_freedos_stub_hd_for_int19();
        }
        self.seed_bda_disk_equipment()?;
        let measure = self.measure_guest_boot(GuestBootMedia::ActiveVbr, max_steps)?;
        Ok(self.finish_os_measure(
            GuestOsMeasureKind::FreeDosLike,
            measure,
            "Host MBR→VBR chain measured on FreeDOS stub — does NOT claim FreeDOS prompt or SeaBIOS INT19.",
            vec![
                "Active-partition VBR executed via host chain (not guest MBR code)",
                "Synthetic VBR has no COMMAND.COM / FreeDOS kernel",
                "Guest INT 13h still needs SeaBIOS (host subset is not an IVT body)",
                "No claim of FreeDOS prompt",
            ],
            vec![
                "VBR-chain handoff classifies past media-attached-beyond-reboot-loop when IVT is present",
            ],
            FreedosHandoff::ActiveVbr,
        ))
    }

    /// Attach FAT12 FreeDOS stub HD (`KERNEL.SYS` root name) and measure VBR chain.
    ///
    /// Past [`FreedosNextGap::ExecutedVbrMissingCommand`]: host FAT12 locate finds
    /// `KERNEL.SYS` → [`FreedosNextGap::KernelNameLocatedMissingLoad`].
    ///
    /// Still **not** a FreeDOS prompt — name locate ≠ cluster load/exec.
    pub fn measure_freedos_fat12_root(
        &mut self,
        max_steps: u64,
    ) -> Result<GuestOsMeasure, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_freedos_fat12_hd_for_int19();
        }
        self.seed_bda_disk_equipment()?;
        let measure = self.measure_guest_boot(GuestBootMedia::ActiveVbr, max_steps)?;
        Ok(self.finish_os_measure(
            GuestOsMeasureKind::FreeDosLike,
            measure,
            "Host FAT12 root KERNEL.SYS name locate — does NOT claim FreeDOS prompt or kernel exec.",
            vec![
                "FAT12 BPB + root directory walked on host (not guest INT13)",
                "KERNEL.SYS directory name present; clusters not loaded/executed",
                "Guest INT 13h still needs SeaBIOS (host subset is not an IVT body)",
                "No claim of FreeDOS prompt",
            ],
            vec![
                "FAT12 name locate classifies past executed-vbr-missing-command when IVT is present",
            ],
            FreedosHandoff::ActiveVbr,
        ))
    }

    /// Attach a synthetic Linux serial-path stub (if no IDE yet) and measure.
    ///
    /// Captures COM1 from a guest that prints a short banner then `HLT`. Does
    /// **not** load a bzImage, enter protected mode, or claim Linux boot / M2 exit.
    ///
    /// For host-side bzImage triage use [`inspect_linux_boot_protocol_header`] /
    /// [`classify_bzimage_early`] / [`Self::load_bzimage_realmode_setup`].
    pub fn measure_linux_serial_path(
        &mut self,
        max_steps: u64,
    ) -> Result<GuestOsMeasure, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_ide_image(synthetic_linux_serial_stub_disk());
        }
        let measure = self.measure_guest_boot(GuestBootMedia::IdePrefer, max_steps)?;
        Ok(self.finish_os_measure(
            GuestOsMeasureKind::LinuxSerialPath,
            measure,
            "Synthetic serial-printing stub only — does NOT claim Linux boot, userspace, or Milestone 2 exit.",
            vec![
                "No bzImage / vmlinux fixture vendored or loaded",
                "No Linux boot protocol (real-mode setup, protected-mode jump)",
                "No earlyprintk / 8250 console driver path through a real kernel",
                "Missing SeaBIOS INT 13h guest path for disked bootloaders",
                "Protected-mode / paging / CPUID gaps may still block real kernels",
                "Header inspect + classify_bzimage_early available; does not execute setup code",
            ],
            vec![],
            FreedosHandoff::MbrSector,
        ))
    }

    /// Attach El Torito no-emul CD (if needed) and measure first-failure.
    ///
    /// Host [`Self::load_eltorito_to_7c00`] then [`Self::probe_post`]. Does
    /// **not** claim Linux serial shell or SeaBIOS CD INT 13h success.
    pub fn measure_linux_with_eltorito_media(
        &mut self,
        max_steps: u64,
    ) -> Result<GuestOsMeasure, MachineError> {
        if !self.ide.is_atapi_cdrom() || self.ide.atapi_medium_image().is_none() {
            self.attach_atapi_cdrom_image(synthetic_eltorito_linux_hlt_iso());
        }
        let measure = self.measure_guest_boot(GuestBootMedia::ElTorito, max_steps)?;
        Ok(self.finish_os_measure(
            GuestOsMeasureKind::LinuxSerialPath,
            measure,
            "El Torito no-emul media measured — does NOT claim Linux serial shell or SeaBIOS CD stack.",
            vec![
                "Synthetic El Torito HLT boot image only (not a bzImage / kernel)",
                "Host load_eltorito_to_7c00 — not guest INT 13h CD path",
                "No Linux boot protocol / earlyprintk through a real kernel",
                "No claim of Linux serial shell",
            ],
            vec![
                "classify_linux_media_boot / classify_eltorito_media_boot for candidacy",
            ],
            FreedosHandoff::MbrSector,
        ))
    }

    /// Copy the real-mode portion of a host bzImage into guest RAM at `dest`.
    ///
    /// Copies `(setup_sects+1)×512` bytes (with `setup_sects==0` meaning 4) after
    /// [`classify_bzimage_early`] succeeds with `SetupLoadable`. Default load
    /// address is [`LINUX_REALMODE_LOAD_ADDR`] (`0x90000`).
    ///
    /// Does **not** arm `CS:IP`, enter protected mode, load the high kernel, or
    /// claim a serial shell. Spec: Linux `Documentation/x86/boot.rst`.
    pub fn load_bzimage_realmode_setup(
        &mut self,
        image: &[u8],
        dest: u64,
    ) -> Result<BzImageEarlyClass, BzImageLoadError> {
        let class = classify_bzimage_early(image);
        match &class {
            BzImageEarlyClass::BadHeader(e) => return Err(BzImageLoadError::Classify(*e)),
            BzImageEarlyClass::IncompleteSetup { have, need, .. } => {
                return Err(BzImageLoadError::IncompleteSetup {
                    have: *have,
                    need: *need,
                });
            }
            BzImageEarlyClass::SetupLoadable { setup_sects, .. } => {
                let bytes = linux_realmode_bytes(*setup_sects);
                let end = dest
                    .checked_add(bytes as u64)
                    .ok_or(BzImageLoadError::RamTooSmall)?;
                if end > self.mem.ram_len() as u64 {
                    return Err(BzImageLoadError::RamTooSmall);
                }
                for (i, b) in image[..bytes].iter().enumerate() {
                    self.mem
                        .write_u8(dest + i as u64, *b)
                        .map_err(|_| BzImageLoadError::RamTooSmall)?;
                }
            }
        }
        Ok(class)
    }

    /// Arm `CS:IP` at the Linux real-mode setup entry (offset `0x200` from `dest`).
    ///
    /// Spec: Linux `Documentation/x86/boot.rst` — for protocol ≥ 2.00 the entry
    /// is at offset `0x200` from the start of the real-mode kernel. Uses
    /// `CS = dest>>4`, `IP = 0x200`. Does **not** execute setup or claim a shell.
    pub fn arm_bzimage_realmode_entry(&mut self, dest: u64) -> Result<(), BzImageLoadError> {
        if dest & 0xF != 0 {
            return Err(BzImageLoadError::RamTooSmall);
        }
        if dest > u64::from(u16::MAX) << 4 {
            return Err(BzImageLoadError::RamTooSmall);
        }
        let cs = (dest >> 4) as u16;
        self.cpu.cs = x86_core::SegmentReg::real_mode_code(cs);
        self.cpu.set_ip16(0x0200);
        Ok(())
    }

    /// Load synthetic bzImage setup, arm entry at `+0x200`, measure first-failure (R15).
    ///
    /// Advances Linux classify past El Torito/serial stub halt toward
    /// [`LinuxNextGap::SetupExecutedMissingProtectedKernel`].
    ///
    /// Does **not** claim a Linux serial shell or protected-mode kernel boot.
    pub fn measure_linux_bzimage_setup_entry(
        &mut self,
        max_steps: u64,
    ) -> Result<(GuestOsMeasure, LinuxNextGap), MachineError> {
        let image = synthetic_linux_bzimage_setup_hlt();
        // Need ≥ ~0x90400 RAM for classic load address + setup.
        if self.mem.ram_len() < (LINUX_REALMODE_LOAD_ADDR as usize) + linux_realmode_bytes(1) {
            return Err(MachineError::MbrRamTooSmall);
        }
        // Host INT13 AH=41 probe in finish_os_measure CF-masks halt without media;
        // attach a tiny signed sector so the setup-entry halt stays SyntheticHalt.
        if !self.ide.present || self.ide.image.is_empty() {
            let mut sector = vec![0x90u8; crate::mbr::MBR_SECTOR_SIZE];
            sector[0] = 0xF4;
            sector[510] = crate::mbr::MBR_SIGNATURE_LO;
            sector[511] = crate::mbr::MBR_SIGNATURE_HI;
            self.attach_ide_image(sector);
        }
        self.load_bzimage_realmode_setup(&image, LINUX_REALMODE_LOAD_ADDR)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        self.arm_bzimage_realmode_entry(LINUX_REALMODE_LOAD_ADDR)
            .map_err(|_| MachineError::MbrRamTooSmall)?;
        let mut checkpoints = vec![
            GuestBootCheckpoint::MediaLoaded,
            GuestBootCheckpoint::CsIpArmed,
            GuestBootCheckpoint::ProbeStarted,
        ];
        let report = self.probe_post(max_steps);
        let com1 = report.com1.clone();
        let debug = report.debug.clone();
        if !com1.is_empty() || !debug.is_empty() {
            checkpoints.push(GuestBootCheckpoint::SerialObserved);
        }
        let vga_summary = vga_text_summary(self);
        if !vga_summary.is_empty() {
            checkpoints.push(GuestBootCheckpoint::VgaObserved);
        }
        checkpoints.push(GuestBootCheckpoint::StopRecorded);
        let measure = GuestBootMeasure {
            version: GUEST_BOOT_MEASURE_VERSION,
            media: GuestBootMedia::BzImageSetup,
            checkpoints,
            com1,
            debug,
            vga_summary,
            report,
        };
        let os = self.finish_os_measure(
            GuestOsMeasureKind::LinuxSerialPath,
            measure,
            "bzImage real-mode setup entry measured — does NOT claim Linux serial shell or PM kernel.",
            vec![
                "Synthetic setup entry at +0x200 (jmp to COM1 LX + HLT) only",
                "No protected-mode kernel / code32_start jump",
                "No earlyprintk through a real kernel",
                "No claim of Linux serial shell",
            ],
            vec![
                "classify_linux_next_gap → setup-executed-missing-protected-kernel when armed",
            ],
            FreedosHandoff::MbrSector,
        );
        let media = classify_linux_media_boot(self, Some(&image));
        let gap = classify_linux_next_gap(&os.first_failure, &media, true);
        Ok((os, gap))
    }
}

/// Compact non-blank VGA text rows (`row:text`), empty when the buffer is blank.
fn vga_text_summary(machine: &Machine) -> String {
    let mut parts = Vec::new();
    for row in 0..25usize {
        let mut line = String::new();
        let mut blank = true;
        for col in 0..80usize {
            let ch = machine.vga.char_at(row, col).unwrap_or(b' ');
            if ch != b' ' && ch != 0 {
                blank = false;
            }
            if (0x20..=0x7E).contains(&ch) {
                line.push(ch as char);
            } else {
                line.push('.');
            }
        }
        if !blank {
            parts.push(format!("{row}:{}", line.trim_end()));
        }
        if parts.len() >= 3 {
            break;
        }
    }
    parts.join(" | ")
}

/// FreeDOS-*like* IDE image: MBR prints `FD` to COM1 + VGA 'F', then HLT;
/// LBA1 holds an ASCII payload marker (not executed by this fixture).
pub fn synthetic_freedos_like_disk() -> Vec<u8> {
    let mut img = vec![0u8; 4 * crate::mbr::MBR_SECTOR_SIZE];
    let mut mbr = vec![0x90u8; crate::mbr::MBR_SECTOR_SIZE];
    // mov dx,0x3F8; mov al,'F'; out dx,al; mov al,'D'; out dx,al;
    // mov ax,0xB800; mov es,ax; xor di,di; mov al,'F'; mov ah,0x07; stosw; hlt
    let code: &[u8] = &[
        0xBA, 0xF8, 0x03, // mov dx, 0x03F8
        0xB0, b'F', // mov al, 'F'
        0xEE, // out dx, al
        0xB0, b'D', // mov al, 'D'
        0xEE, // out dx, al
        0xB8, 0x00, 0xB8, // mov ax, 0xB800
        0x8E, 0xC0, // mov es, ax
        0x31, 0xFF, // xor di, di
        0xB0, b'F', // mov al, 'F'
        0xB4, 0x07, // mov ah, 0x07
        0xAB, // stosw
        0xF4, // hlt
    ];
    mbr[..code.len()].copy_from_slice(code);
    mbr[510] = crate::mbr::MBR_SIGNATURE_LO;
    mbr[511] = crate::mbr::MBR_SIGNATURE_HI;
    img[..crate::mbr::MBR_SECTOR_SIZE].copy_from_slice(&mbr);
    let marker = b"FREEDOS-LIKE-PAYLOAD\0";
    img[crate::mbr::MBR_SECTOR_SIZE..crate::mbr::MBR_SECTOR_SIZE + marker.len()]
        .copy_from_slice(marker);
    img
}

/// Linux serial-path stub: MBR prints `LX\r\n` to COM1 then HLT (no kernel).
///
/// The CRLF is a tiny earlyprintk-shaped line ending for serial capture; it is
/// **not** a Linux boot-protocol or earlyprintk driver claim.
pub fn synthetic_linux_serial_stub_disk() -> Vec<u8> {
    let mut img = vec![0u8; 2 * crate::mbr::MBR_SECTOR_SIZE];
    let mut mbr = vec![0x90u8; crate::mbr::MBR_SECTOR_SIZE];
    let code: &[u8] = &[
        0xBA, 0xF8, 0x03, // mov dx, 0x03F8
        0xB0, b'L', // mov al, 'L'
        0xEE, // out dx, al
        0xB0, b'X', // mov al, 'X'
        0xEE, // out dx, al
        0xB0, b'\r', // mov al, CR
        0xEE,  // out dx, al
        0xB0, b'\n', // mov al, LF
        0xEE,  // out dx, al
        0xF4,  // hlt
    ];
    mbr[..code.len()].copy_from_slice(code);
    mbr[510] = crate::mbr::MBR_SIGNATURE_LO;
    mbr[511] = crate::mbr::MBR_SIGNATURE_HI;
    img[..crate::mbr::MBR_SECTOR_SIZE].copy_from_slice(&mbr);
    let marker = b"LINUX-SERIAL-STUB\0";
    img[crate::mbr::MBR_SECTOR_SIZE..crate::mbr::MBR_SECTOR_SIZE + marker.len()]
        .copy_from_slice(marker);
    img
}

/// Synthetic El Torito no-emul ISO with a HLT boot image (Linux-media measure fixture).
///
/// Spec: El Torito 1.0 Validation + Default Entry (`88h`, media `00h`). Not a
/// bzImage and **not** a Linux serial shell.
pub fn synthetic_eltorito_linux_hlt_iso() -> Vec<u8> {
    use firmware_interface::{
        EL_TORITO_BOOTABLE, EL_TORITO_BOOT_SYSTEM_ID, EL_TORITO_KEY_55, EL_TORITO_KEY_AA,
        EL_TORITO_MEDIA_NO_EMUL, EL_TORITO_PLATFORM_X86, EL_TORITO_SECTOR_BYTES,
        EL_TORITO_VALIDATION_HEADER_ID, ISO9660_STANDARD_ID, ISO9660_VD_BOOT_RECORD,
        ISO9660_VD_TERMINATOR,
    };
    fn write_iso_sector(img: &mut [u8], lba: u32, data: &[u8]) {
        let start = lba as usize * EL_TORITO_SECTOR_BYTES;
        img[start..start + data.len()].copy_from_slice(data);
    }
    let mut img = vec![0u8; 32 * EL_TORITO_SECTOR_BYTES];
    let mut pvd = vec![0u8; EL_TORITO_SECTOR_BYTES];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(ISO9660_STANDARD_ID);
    pvd[6] = 1;
    write_iso_sector(&mut img, 16, &pvd);

    let mut br = vec![0u8; EL_TORITO_SECTOR_BYTES];
    br[0] = ISO9660_VD_BOOT_RECORD;
    br[1..6].copy_from_slice(ISO9660_STANDARD_ID);
    br[6] = 1;
    br[7..7 + EL_TORITO_BOOT_SYSTEM_ID.len()].copy_from_slice(EL_TORITO_BOOT_SYSTEM_ID);
    let catalog_lba = 20u32;
    br[0x47..0x4B].copy_from_slice(&catalog_lba.to_le_bytes());
    write_iso_sector(&mut img, 17, &br);

    let mut term = vec![0u8; EL_TORITO_SECTOR_BYTES];
    term[0] = ISO9660_VD_TERMINATOR;
    term[1..6].copy_from_slice(ISO9660_STANDARD_ID);
    term[6] = 1;
    write_iso_sector(&mut img, 18, &term);

    let mut cat = vec![0u8; EL_TORITO_SECTOR_BYTES];
    let mut validation = [0u8; 32];
    validation[0] = EL_TORITO_VALIDATION_HEADER_ID;
    validation[1] = EL_TORITO_PLATFORM_X86;
    validation[30] = EL_TORITO_KEY_55;
    validation[31] = EL_TORITO_KEY_AA;
    let mut sum = 0u16;
    for i in (0..32).step_by(2) {
        if i == 28 {
            continue;
        }
        sum = sum.wrapping_add(u16::from_le_bytes([validation[i], validation[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    validation[28..30].copy_from_slice(&checksum.to_le_bytes());
    cat[0..32].copy_from_slice(&validation);
    cat[32] = EL_TORITO_BOOTABLE;
    cat[33] = EL_TORITO_MEDIA_NO_EMUL;
    cat[38..40].copy_from_slice(&4u16.to_le_bytes());
    cat[40..44].copy_from_slice(&24u32.to_le_bytes());
    write_iso_sector(&mut img, catalog_lba, &cat);

    // Boot image: COM1 "LX" + HLT (linux-media first-failure fixture).
    let mut boot = vec![0x90u8; EL_TORITO_SECTOR_BYTES];
    let code: &[u8] = &[
        0xBA, 0xF8, 0x03, // mov dx, 0x03F8
        0xB0, b'L', //
        0xEE, //
        0xB0, b'X', //
        0xEE, //
        0xF4, // hlt
    ];
    boot[..code.len()].copy_from_slice(code);
    write_iso_sector(&mut img, 24, &boot);
    img
}

/// Synthetic El Torito no-emul ISO whose boot image is a bzImage-shaped setup stub.
///
/// Catalog → load peeks as [`ElToritoPayloadClass::NoEmulBzImage`]. Not a real
/// kernel and **not** a Linux serial shell.
pub fn synthetic_eltorito_bzimage_iso() -> Vec<u8> {
    use firmware_interface::EL_TORITO_SECTOR_BYTES;
    let mut img = synthetic_eltorito_linux_hlt_iso();
    let boot = synthetic_linux_bzimage_setup_hlt();
    let start = 24 * EL_TORITO_SECTOR_BYTES;
    let n = boot.len().min(EL_TORITO_SECTOR_BYTES);
    img[start..start + n].copy_from_slice(&boot[..n]);
    // Clear remainder of the ISO sector so leftover HLT stub bytes do not linger.
    for b in &mut img[start + n..start + EL_TORITO_SECTOR_BYTES] {
        *b = 0;
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbr::{MBR_SECTOR_SIZE, MBR_SIGNATURE_HI, MBR_SIGNATURE_LO};
    use crate::post_probe::{PostFailureKind, PostStopReason};
    use devices::FDC_1440_IMAGE_SIZE;
    use firmware_interface::{
        EL_TORITO_BOOTABLE, EL_TORITO_BOOT_SYSTEM_ID, EL_TORITO_KEY_55, EL_TORITO_KEY_AA,
        EL_TORITO_MEDIA_NO_EMUL, EL_TORITO_PLATFORM_X86, EL_TORITO_SECTOR_BYTES,
        EL_TORITO_VALIDATION_HEADER_ID, ISO9660_STANDARD_ID, ISO9660_VD_BOOT_RECORD,
        ISO9660_VD_TERMINATOR,
    };

    fn synthetic_mbr_hlt() -> Vec<u8> {
        let mut sector = vec![0x90u8; MBR_SECTOR_SIZE];
        sector[0] = 0xF4; // HLT
        sector[510] = MBR_SIGNATURE_LO;
        sector[511] = MBR_SIGNATURE_HI;
        sector
    }

    fn synthetic_mbr_ud() -> Vec<u8> {
        let mut sector = vec![0x90u8; MBR_SECTOR_SIZE];
        // WAIT (0x9B) — valid encoding, unimplemented in this tree (POST probe uses it).
        sector[0] = 0x9B;
        sector[510] = MBR_SIGNATURE_LO;
        sector[511] = MBR_SIGNATURE_HI;
        sector
    }

    /// COM1 THR write then HLT — serial-path checkpoint signal for FreeDOS/Linux prep.
    fn synthetic_mbr_serial_then_hlt() -> Vec<u8> {
        let mut sector = vec![0x90u8; MBR_SECTOR_SIZE];
        // mov dx, 0x3F8 ; mov al, 'F' ; out dx, al ; hlt
        let code: &[u8] = &[
            0xBA, 0xF8, 0x03, // mov dx, 0x03F8
            0xB0, b'F', // mov al, 'F'
            0xEE, // out dx, al
            0xF4, // hlt
        ];
        sector[..code.len()].copy_from_slice(code);
        sector[510] = MBR_SIGNATURE_LO;
        sector[511] = MBR_SIGNATURE_HI;
        sector
    }

    fn write_iso_sector(img: &mut [u8], lba: u32, data: &[u8]) {
        let start = lba as usize * EL_TORITO_SECTOR_BYTES;
        img[start..start + data.len()].copy_from_slice(data);
    }

    fn synthetic_eltorito_hlt_iso() -> Vec<u8> {
        let mut img = vec![0u8; 32 * EL_TORITO_SECTOR_BYTES];
        let mut pvd = vec![0u8; EL_TORITO_SECTOR_BYTES];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        pvd[6] = 1;
        write_iso_sector(&mut img, 16, &pvd);

        let mut br = vec![0u8; EL_TORITO_SECTOR_BYTES];
        br[0] = ISO9660_VD_BOOT_RECORD;
        br[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        br[6] = 1;
        br[7..7 + EL_TORITO_BOOT_SYSTEM_ID.len()].copy_from_slice(EL_TORITO_BOOT_SYSTEM_ID);
        let catalog_lba = 20u32;
        br[0x47..0x4B].copy_from_slice(&catalog_lba.to_le_bytes());
        write_iso_sector(&mut img, 17, &br);

        let mut term = vec![0u8; EL_TORITO_SECTOR_BYTES];
        term[0] = ISO9660_VD_TERMINATOR;
        term[1..6].copy_from_slice(ISO9660_STANDARD_ID);
        term[6] = 1;
        write_iso_sector(&mut img, 18, &term);

        let mut cat = vec![0u8; EL_TORITO_SECTOR_BYTES];
        let mut validation = [0u8; 32];
        validation[0] = EL_TORITO_VALIDATION_HEADER_ID;
        validation[1] = EL_TORITO_PLATFORM_X86;
        validation[30] = EL_TORITO_KEY_55;
        validation[31] = EL_TORITO_KEY_AA;
        let mut sum = 0u16;
        for i in (0..32).step_by(2) {
            if i == 28 {
                continue;
            }
            sum = sum.wrapping_add(u16::from_le_bytes([validation[i], validation[i + 1]]));
        }
        let checksum = 0u16.wrapping_sub(sum);
        validation[28..30].copy_from_slice(&checksum.to_le_bytes());
        cat[0..32].copy_from_slice(&validation);
        cat[32] = EL_TORITO_BOOTABLE;
        cat[33] = EL_TORITO_MEDIA_NO_EMUL;
        cat[38..40].copy_from_slice(&4u16.to_le_bytes());
        cat[40..44].copy_from_slice(&24u32.to_le_bytes());
        write_iso_sector(&mut img, catalog_lba, &cat);

        let mut boot = vec![0x90u8; EL_TORITO_SECTOR_BYTES];
        boot[0] = 0xF4;
        write_iso_sector(&mut img, 24, &boot);
        img
    }

    /// Measure-first: HLT boot sector stops with an honest halt reason.
    #[test]
    fn measure_guest_boot_hlt_records_halt() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr_hlt());
        let measure = m
            .measure_guest_boot(GuestBootMedia::IdePrefer, 64)
            .expect("measure");
        assert_eq!(measure.version, GUEST_BOOT_MEASURE_VERSION);
        assert!(matches!(measure.report.stop, PostStopReason::Halted));
        assert!(measure
            .checkpoints
            .contains(&GuestBootCheckpoint::MediaLoaded));
        assert!(measure
            .checkpoints
            .contains(&GuestBootCheckpoint::StopRecorded));
        let text = measure.to_string();
        assert!(text.contains("guest-measure-v2:"));
        assert!(text.contains("not a boot-success claim"));
        assert!(text.contains("halted"));
    }

    /// Measure-first: first failure on an undefined opcode is recorded.
    #[test]
    fn measure_guest_boot_records_first_failure() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr_ud());
        let measure = m
            .measure_guest_boot(GuestBootMedia::IdePrefer, 64)
            .expect("measure");
        match &measure.report.stop {
            PostStopReason::Failure(f) => match &f.kind {
                PostFailureKind::UnsupportedOpcode(_)
                | PostFailureKind::UnsupportedEncoding(_)
                | PostFailureKind::ArchFault { .. } => {}
                other => panic!("unexpected failure kind: {other:?}"),
            },
            other => panic!("expected failure, got {other:?}"),
        }
        assert_eq!(measure.report.stop_site.cs, 0);
        assert_eq!(measure.report.stop_site.eip, 0x7C00);
        assert_eq!(measure.stop_class(), "first-failure");
    }

    #[test]
    fn measure_guest_boot_floppy_first() {
        let mut floppy = vec![0u8; FDC_1440_IMAGE_SIZE];
        floppy[..MBR_SECTOR_SIZE].copy_from_slice(&synthetic_mbr_hlt());
        let mut m = Machine::with_floppy(64 * 1024, floppy).expect("floppy");
        // Attach decoy IDE that would win under IdePrefer.
        m.attach_ide_image(synthetic_mbr_ud());
        let measure = m
            .measure_guest_boot(GuestBootMedia::FloppyFirst, 64)
            .expect("floppy measure");
        assert!(matches!(measure.report.stop, PostStopReason::Halted));
        assert_eq!(measure.media, GuestBootMedia::FloppyFirst);
    }

    #[test]
    fn measure_guest_boot_no_media_errors() {
        let mut m = Machine::new(64 * 1024);
        assert!(matches!(
            m.measure_guest_boot(GuestBootMedia::IdePrefer, 16),
            Err(MachineError::NoBootMedia)
        ));
    }

    /// v2 serial path: guest OUT to COM1 is captured as a checkpoint.
    #[test]
    fn measure_guest_boot_v2_captures_com1_serial() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr_serial_then_hlt());
        let measure = m
            .measure_guest_boot(GuestBootMedia::IdePrefer, 64)
            .expect("measure");
        assert!(measure.serial_captured());
        assert_eq!(measure.com1, "F");
        assert!(measure
            .checkpoints
            .contains(&GuestBootCheckpoint::SerialObserved));
        assert!(matches!(measure.report.stop, PostStopReason::Halted));
        let text = measure.to_string();
        assert!(text.contains("serial-observed"));
        assert!(text.contains("com1=\"F\""));
    }

    /// v2 El Torito media path uses host no-emul handoff then probe.
    #[test]
    fn measure_guest_boot_eltorito_hlt() {
        let mut m = Machine::new(64 * 1024);
        m.attach_atapi_cdrom_image(synthetic_eltorito_hlt_iso());
        let measure = m
            .measure_guest_boot(GuestBootMedia::ElTorito, 64)
            .expect("eltorito measure");
        assert_eq!(measure.media, GuestBootMedia::ElTorito);
        assert!(matches!(measure.report.stop, PostStopReason::Halted));
        assert!(measure
            .checkpoints
            .contains(&GuestBootCheckpoint::CsIpArmed));
    }

    /// FreeDOS-like harness: serial + VGA checkpoints; never claims a prompt.
    #[test]
    fn measure_freedos_like_serial_and_vga() {
        let mut m = Machine::new(64 * 1024);
        let report = m.measure_freedos_like(128).expect("freedos-like");
        assert_eq!(report.version, GUEST_OS_MEASURE_VERSION);
        assert_eq!(report.kind, GuestOsMeasureKind::FreeDosLike);
        assert_eq!(report.measure.com1, "FD");
        assert!(report.measure.serial_captured());
        assert!(report.measure.vga_captured());
        assert!(report
            .measure
            .checkpoints
            .contains(&GuestBootCheckpoint::VgaObserved));
        assert!(matches!(report.measure.report.stop, PostStopReason::Halted));
        assert_eq!(report.first_failure, GuestFirstFailureClass::SyntheticHalt);
        assert_eq!(report.first_failure.tag(), "synthetic-halt");
        assert_eq!(report.next_gap, FreedosNextGap::GuestInt13IvtMissing);
        assert_eq!(report.next_gap.tag(), "guest-int13-ivt-missing");
        let text = report.to_string();
        assert!(text.contains("NOT an OS boot"));
        assert!(text.contains("does NOT claim a FreeDOS prompt"));
        assert!(text.contains("freedos-like"));
        assert!(text.contains("first-failure=synthetic-halt"));
        assert!(text.contains("next-gap=guest-int13-ivt-missing"));
        assert!(report.gaps.iter().any(|g| g.contains("prompt")));
        assert!(report
            .gaps
            .iter()
            .any(|g| g.contains("not FreeDOS/Linux progress")));
        assert!(report
            .host_notes
            .iter()
            .any(|n| n.contains("BDA 0040:0010/0075")));
        assert!(report
            .host_notes
            .iter()
            .any(|n| n.contains("IVT INT 13h null")));
        assert!(text.contains("host-notes="));
        // Payload marker exists on disk but is not a boot claim.
        assert!(m.ide.image[MBR_SECTOR_SIZE..].starts_with(b"FREEDOS-LIKE-PAYLOAD"));
        // BDA disk equipment seeded for the measure path.
        assert_eq!(m.mem.read_u8(BDA_HD_COUNT).unwrap(), 1);
        assert_eq!(m.mem.read_u8(BDA_EQUIPMENT).unwrap(), m.equipment_byte());
    }

    /// Next-gap: BDA mismatch wins over IVT-null when equipment bytes disagree.
    #[test]
    fn classify_freedos_next_gap_bda_mismatch() {
        let mut m = Machine::new(64 * 1024);
        m.attach_ide_image(synthetic_freedos_like_disk());
        m.seed_bda_disk_equipment().unwrap();
        // Corrupt HD count after seeding.
        m.mem.write_u8(BDA_HD_COUNT, 0).unwrap();
        let probe = Int13ProbeSnapshot {
            dl: 0x80,
            ah: 0x01,
            cf: false,
        };
        let gap = classify_freedos_next_gap(&m, &GuestFirstFailureClass::SyntheticHalt, &probe);
        assert_eq!(gap, FreedosNextGap::BdaDiskMismatch);
    }

    /// Next-gap: installed INT 13h IVT pointer advances past IVT-missing to image/firmware.
    #[test]
    fn classify_freedos_next_gap_with_ivt_pointer() {
        let mut m = Machine::new(64 * 1024);
        m.attach_ide_image(synthetic_freedos_like_disk());
        m.seed_bda_disk_equipment().unwrap();
        m.install_int13_ivt_pointer(0xF000, 0xE000).unwrap();
        let probe = Int13ProbeSnapshot {
            dl: 0x80,
            ah: 0x01,
            cf: false,
        };
        let gap = classify_freedos_next_gap(&m, &GuestFirstFailureClass::SyntheticHalt, &probe);
        assert_eq!(gap, FreedosNextGap::RealImageAndFirmware);
    }

    /// BDA seed: floppy media flips equipment diskette bits; HD count tracks IDE.
    #[test]
    fn seed_bda_disk_equipment_tracks_media() {
        let mut bare = Machine::new(64 * 1024);
        bare.seed_bda_disk_equipment().unwrap();
        assert_eq!(bare.mem.read_u8(BDA_HD_COUNT).unwrap(), 0);
        assert_eq!(
            bare.mem.read_u8(BDA_EQUIPMENT).unwrap() & 0x01,
            0,
            "no floppy bit without media"
        );

        let mut floppy = vec![0u8; FDC_1440_IMAGE_SIZE];
        floppy[..MBR_SECTOR_SIZE].copy_from_slice(&synthetic_mbr_hlt());
        let mut m = Machine::with_floppy(64 * 1024, floppy).expect("floppy");
        m.attach_ide_image(synthetic_mbr_hlt());
        m.seed_bda_disk_equipment().unwrap();
        assert_eq!(m.mem.read_u8(BDA_HD_COUNT).unwrap(), 1);
        assert_eq!(m.mem.read_u8(BDA_EQUIPMENT).unwrap(), m.equipment_byte());
        assert_ne!(m.mem.read_u8(BDA_EQUIPMENT).unwrap() & 0x01, 0);
    }

    /// FreeDOS-like first-failure class for an unimplemented opcode fixture.
    #[test]
    fn measure_freedos_like_classifies_unsupported_encoding() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr_ud());
        let report = m.measure_freedos_like(64).expect("freedos-like failure");
        assert!(matches!(
            report.first_failure,
            GuestFirstFailureClass::UnsupportedOpcode { .. }
                | GuestFirstFailureClass::UnsupportedEncoding { .. }
                | GuestFirstFailureClass::ArchFault { .. }
        ));
        assert_eq!(
            report.first_failure.tag(),
            match &report.first_failure {
                GuestFirstFailureClass::UnsupportedOpcode { .. } => "unsupported-opcode",
                GuestFirstFailureClass::UnsupportedEncoding { .. } => "unsupported-encoding",
                GuestFirstFailureClass::ArchFault { .. } => "arch-fault",
                other => panic!("unexpected class {other}"),
            }
        );
        assert!(report.gaps.iter().any(|g| g.contains("First failure")));
        assert_eq!(report.failure_bucket, "decode-ud");
        assert_eq!(report.failure_site.eip, 0x7C00);
        let text = report.to_string();
        assert!(text.contains("NOT an OS boot"));
        assert!(text.contains("bucket=decode-ud"));
        assert!(!text.contains("FreeDOS prompt reached"));
    }

    /// Linux serial-path harness: COM1 banner + documented gaps; not M2 exit.
    #[test]
    fn measure_linux_serial_path_captures_com1_and_gaps() {
        let mut m = Machine::new(64 * 1024);
        let report = m.measure_linux_serial_path(64).expect("linux-serial");
        assert_eq!(report.kind, GuestOsMeasureKind::LinuxSerialPath);
        assert_eq!(report.measure.com1, "LX\r\n");
        assert_eq!(report.first_failure, GuestFirstFailureClass::SyntheticHalt);
        assert!(report.gaps.iter().any(|g| g.contains("bzImage")));
        assert!(report.gaps.iter().any(|g| g.contains("boot protocol")));
        assert!(report
            .host_notes
            .iter()
            .any(|n| n.contains("classify_bzimage_early")));
        let text = report.to_string();
        assert!(text.contains("NOT Milestone 2 exit"));
        assert!(text.contains("linux-serial-path"));
        assert!(text.contains("first-failure=synthetic-halt"));
        assert!(text.contains("bucket=halted"));
        assert_eq!(report.failure_bucket, "halted");
        assert!(!report.int13_probe.failed());
        assert!(matches!(report.measure.report.stop, PostStopReason::Halted));
    }

    /// Linux boot-protocol header inspect: HdrS + version + loadflags (no bzImage).
    #[test]
    fn inspect_linux_boot_protocol_header_accepts_synthetic() {
        let buf = synthetic_linux_boot_protocol_header(4, 0x020F, 0x01, 0x0010_0000);
        let hdr = inspect_linux_boot_protocol_header(&buf).expect("header");
        assert_eq!(hdr.setup_sects, 4);
        assert_eq!(hdr.boot_flag, LINUX_BOOT_FLAG_AA55);
        assert_eq!(hdr.header_magic, LINUX_BOOT_HEADER_MAGIC);
        assert_eq!(hdr.version, 0x020F);
        assert!(hdr.loaded_high());
        assert_eq!(hdr.code32_start, Some(0x0010_0000));
    }

    /// Linux boot-protocol inspect rejects truncated / bad magic / bad AA55.
    #[test]
    fn inspect_linux_boot_protocol_header_rejects_bad() {
        assert_eq!(
            inspect_linux_boot_protocol_header(&[0u8; 16]),
            Err(LinuxBootProtocolError::Truncated)
        );
        let mut bad_flag = synthetic_linux_boot_protocol_header(1, 0x0200, 0, 0);
        bad_flag[0x1FE] = 0;
        assert_eq!(
            inspect_linux_boot_protocol_header(&bad_flag),
            Err(LinuxBootProtocolError::BadBootFlag)
        );
        let mut bad_magic = synthetic_linux_boot_protocol_header(1, 0x0200, 0, 0);
        bad_magic[0x202] = b'X';
        assert_eq!(
            inspect_linux_boot_protocol_header(&bad_magic),
            Err(LinuxBootProtocolError::BadMagic)
        );
    }

    /// Spec: Linux boot.rst — early classify names next step (LOADED_HIGH → PM).
    #[test]
    fn classify_bzimage_early_loaded_high() {
        let need = linux_realmode_bytes(4);
        let mut buf = synthetic_linux_boot_protocol_header(4, 0x020F, 0x01, 0x0010_0000);
        buf.resize(need, 0x90);
        buf[0x200] = 0xAB; // marker inside setup
        let class = classify_bzimage_early(&buf);
        match class {
            BzImageEarlyClass::SetupLoadable {
                setup_sects,
                loaded_high,
                next,
                ..
            } => {
                assert_eq!(setup_sects, 4);
                assert!(loaded_high);
                assert_eq!(
                    next,
                    BzImageNextStep::LoadHighProtectedKernel {
                        code32_start: 0x0010_0000
                    }
                );
            }
            other => panic!("unexpected {other}"),
        }
    }

    /// Spec: incomplete setup blob → IncompleteSetup (not a shell claim).
    #[test]
    fn classify_bzimage_early_incomplete_setup() {
        let buf = synthetic_linux_boot_protocol_header(4, 0x0200, 0, 0);
        assert!(buf.len() < linux_realmode_bytes(4));
        match classify_bzimage_early(&buf) {
            BzImageEarlyClass::IncompleteSetup {
                setup_sects,
                have,
                need,
            } => {
                assert_eq!(setup_sects, 4);
                assert_eq!(have, buf.len());
                assert_eq!(need, linux_realmode_bytes(4));
            }
            other => panic!("unexpected {other}"),
        }
    }

    /// Host load copies real-mode setup to 0x90000 without executing.
    #[test]
    fn load_bzimage_realmode_setup_to_90000() {
        let need = linux_realmode_bytes(1);
        let mut buf = synthetic_linux_boot_protocol_header(1, 0x0200, 0, 0);
        buf.resize(need, 0);
        buf[0] = 0xF4;
        buf[512] = 0x5A;
        let mut m = Machine::new(1024 * 1024);
        let class = m
            .load_bzimage_realmode_setup(&buf, LINUX_REALMODE_LOAD_ADDR)
            .expect("load");
        assert_eq!(class.tag(), "setup-loadable");
        assert_eq!(m.mem.read_u8(LINUX_REALMODE_LOAD_ADDR).unwrap(), 0xF4);
        assert_eq!(m.mem.read_u8(LINUX_REALMODE_LOAD_ADDR + 512).unwrap(), 0x5A);
        match class {
            BzImageEarlyClass::SetupLoadable { next, .. } => {
                assert_eq!(next, BzImageNextStep::RunRealModeSetup);
            }
            other => panic!("unexpected {other}"),
        }
    }

    /// Linux serial path classifies unsupported opcode without claiming a shell.
    #[test]
    fn measure_linux_serial_path_classifies_first_failure() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr_ud());
        let report = m
            .measure_linux_serial_path(64)
            .expect("linux-serial failure");
        assert!(matches!(
            report.first_failure,
            GuestFirstFailureClass::UnsupportedOpcode { .. }
                | GuestFirstFailureClass::UnsupportedEncoding { .. }
                | GuestFirstFailureClass::ArchFault { .. }
        ));
        assert_eq!(report.failure_bucket, "decode-ud");
        assert_eq!(report.failure_site.eip, 0x7C00);
        assert!(report.gaps.iter().any(|g| g.contains("bzImage")));
        let text = report.to_string();
        assert!(text.contains("NOT Milestone 2 exit"));
        assert!(text.contains("bucket=decode-ud"));
        assert!(!text.contains("Linux shell"));
    }

    /// FreeDOS-like hang: tiny step budget classifies hang location.
    #[test]
    fn measure_freedos_like_classifies_hang_location() {
        // Infinite jmp $-2 at 7C00 — budget exhausts before HLT.
        let mut sector = vec![0x90u8; MBR_SECTOR_SIZE];
        sector[0] = 0xEB;
        sector[1] = 0xFE; // jmp $
        sector[510] = MBR_SIGNATURE_LO;
        sector[511] = MBR_SIGNATURE_HI;
        let mut m = Machine::with_ide(64 * 1024, sector);
        let report = m.measure_freedos_like(8).expect("hang measure");
        assert_eq!(report.first_failure, GuestFirstFailureClass::StepBudget);
        assert_eq!(report.failure_bucket, "hang");
        assert_eq!(report.failure_site.cs, 0);
        assert_eq!(report.failure_site.eip, 0x7C00);
        let text = report.to_string();
        assert!(text.contains("bucket=hang"));
        assert!(text.contains("site=0000:00007C00"));
        assert!(text.contains("NOT an OS boot"));
    }

    /// INT13 CF class when host AH=41h probe fails (no HD media).
    #[test]
    fn classify_int13_cf_from_probe_snapshot() {
        let mut bare = Machine::new(64 * 1024);
        // Attach a HLT MBR via floppy path? For IdePrefer measure needs IDE.
        // Unit-test classifier directly with a halted report + failed INT13 probe.
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr_hlt());
        let measure = m
            .measure_guest_boot(GuestBootMedia::IdePrefer, 16)
            .expect("hlt");
        let probe = Int13ProbeSnapshot {
            dl: 0x80,
            ah: 0x80,
            cf: true,
        };
        let class = classify_guest_first_failure(&measure.report, Some(&probe));
        assert_eq!(class, GuestFirstFailureClass::Int13Cf { ah: 0x80 });
        assert_eq!(class.bucket(), "int13-cf");
        // Bare machine probe should CF.
        let snap = bare.probe_int13_hd_extensions_status();
        assert!(snap.failed());
    }

    /// Linux serial hang location uses the same bucket vocabulary.
    #[test]
    fn measure_linux_serial_path_classifies_hang() {
        let mut sector = vec![0x90u8; MBR_SECTOR_SIZE];
        sector[0] = 0xEB;
        sector[1] = 0xFE;
        sector[510] = MBR_SIGNATURE_LO;
        sector[511] = MBR_SIGNATURE_HI;
        let mut m = Machine::with_ide(64 * 1024, sector);
        let report = m.measure_linux_serial_path(4).expect("linux hang");
        assert_eq!(report.failure_bucket, "hang");
        assert_eq!(report.first_failure, GuestFirstFailureClass::StepBudget);
        assert!(report.to_string().contains("NOT Milestone 2 exit"));
    }

    /// R13: FreeDOS measure with INT19-candidate media — beyond no-media reboot loop.
    #[test]
    fn measure_freedos_with_bootable_media_classifies_beyond_reboot_loop() {
        let mut m = Machine::new(64 * 1024);
        let report = m
            .measure_freedos_with_bootable_media(64)
            .expect("freedos-with-media");
        assert_eq!(report.version, GUEST_OS_MEASURE_VERSION);
        assert_eq!(report.media_readiness, MediaBootReadiness::Int19Candidate);
        assert_eq!(report.media_readiness.tag(), "int19-candidate");
        assert_eq!(report.first_failure, GuestFirstFailureClass::SyntheticHalt);
        // IVT still null → next-gap names that; media readiness is the reboot-loop signal.
        assert_eq!(report.next_gap, FreedosNextGap::GuestInt13IvtMissing);
        let text = report.to_string();
        assert!(text.contains("media=int19-candidate"));
        assert!(text.contains("NOT an OS boot"));
        assert!(!text.contains("FreeDOS prompt reached"));
        assert!(text.contains("does NOT claim a FreeDOS prompt"));
        assert!(text.contains("guest-os-measure-v8:"));
    }

    /// With IVT stub + INT19 media, next-gap is beyond-reboot-loop (still not a prompt).
    #[test]
    fn classify_freedos_next_gap_media_beyond_reboot_loop() {
        let mut m = Machine::new(64 * 1024);
        m.attach_freedos_stub_hd_for_int19();
        m.seed_bda_disk_equipment().unwrap();
        m.install_int13_ivt_pointer(0xF000, 0xE000).unwrap();
        let probe = Int13ProbeSnapshot {
            dl: 0x80,
            ah: 0x01,
            cf: false,
        };
        let gap = classify_freedos_next_gap(&m, &GuestFirstFailureClass::SyntheticHalt, &probe);
        assert_eq!(gap, FreedosNextGap::MediaAttachedBeyondRebootLoop);
        assert_eq!(gap.tag(), "media-attached-beyond-reboot-loop");
    }

    /// R14: VBR-chain handoff past MediaAttachedBeyondRebootLoop → missing COMMAND.COM.
    #[test]
    fn measure_freedos_vbr_chain_classifies_executed_vbr_missing_command() {
        let mut m = Machine::new(64 * 1024);
        m.attach_freedos_stub_hd_for_int19();
        m.seed_bda_disk_equipment().unwrap();
        m.install_int13_ivt_pointer(0xF000, 0xE000).unwrap();
        let report = m.measure_freedos_vbr_chain(64).expect("vbr-chain");
        assert_eq!(report.version, GUEST_OS_MEASURE_VERSION);
        assert_eq!(report.media_readiness, MediaBootReadiness::Int19Candidate);
        assert_eq!(report.first_failure, GuestFirstFailureClass::SyntheticHalt);
        assert_eq!(report.measure.media, GuestBootMedia::ActiveVbr);
        assert_eq!(report.next_gap, FreedosNextGap::ExecutedVbrMissingCommand);
        assert_eq!(report.next_gap.tag(), "executed-vbr-missing-command");
        assert!(report.measure.com1.contains('F') || report.measure.com1 == "FD");
        let text = report.to_string();
        assert!(text.contains("executed-vbr-missing-command"));
        assert!(!text.contains("FreeDOS prompt reached"));
    }

    /// Spec: ActiveVbr handoff changes next-gap vs MBR-only.
    #[test]
    fn classify_freedos_next_gap_vbr_handoff_missing_command() {
        let mut m = Machine::new(64 * 1024);
        m.attach_freedos_stub_hd_for_int19();
        m.seed_bda_disk_equipment().unwrap();
        m.install_int13_ivt_pointer(0xF000, 0xE000).unwrap();
        let probe = Int13ProbeSnapshot {
            dl: 0x80,
            ah: 0x01,
            cf: false,
        };
        let gap = classify_freedos_next_gap_with_handoff(
            &m,
            &GuestFirstFailureClass::SyntheticHalt,
            &probe,
            FreedosHandoff::ActiveVbr,
        );
        assert_eq!(gap, FreedosNextGap::ExecutedVbrMissingCommand);
    }

    /// R15: FAT12 root `KERNEL.SYS` name → past executed-vbr-missing-command.
    #[test]
    fn measure_freedos_fat12_root_classifies_kernel_name_located() {
        let mut m = Machine::new(64 * 1024);
        m.attach_freedos_fat12_hd_for_int19();
        m.seed_bda_disk_equipment().unwrap();
        m.install_int13_ivt_pointer(0xF000, 0xE000).unwrap();
        let report = m.measure_freedos_fat12_root(64).expect("fat12-root");
        assert_eq!(report.version, GUEST_OS_MEASURE_VERSION);
        assert_eq!(report.media_readiness, MediaBootReadiness::Int19Candidate);
        assert_eq!(report.first_failure, GuestFirstFailureClass::SyntheticHalt);
        assert_eq!(
            report.next_gap,
            FreedosNextGap::KernelNameLocatedMissingLoad
        );
        assert_eq!(report.next_gap.tag(), "kernel-name-located-missing-load");
        assert!(report.measure.com1.contains('F') || report.measure.com1 == "FD");
        let text = report.to_string();
        assert!(text.contains("kernel-name-located-missing-load"));
        assert!(!text.contains("FreeDOS prompt reached"));
    }

    /// Spec: ActiveVbr + FAT12 KERNEL.SYS name advances next-gap.
    #[test]
    fn classify_freedos_next_gap_fat12_kernel_name() {
        let mut m = Machine::new(64 * 1024);
        m.attach_freedos_fat12_hd_for_int19();
        m.seed_bda_disk_equipment().unwrap();
        m.install_int13_ivt_pointer(0xF000, 0xE000).unwrap();
        let probe = Int13ProbeSnapshot {
            dl: 0x80,
            ah: 0x01,
            cf: false,
        };
        let gap = classify_freedos_next_gap_with_handoff(
            &m,
            &GuestFirstFailureClass::SyntheticHalt,
            &probe,
            FreedosHandoff::ActiveVbr,
        );
        assert_eq!(gap, FreedosNextGap::KernelNameLocatedMissingLoad);
    }

    /// Spec: Linux boot.rst — deepen flags missing cmdline / init_size.
    #[test]
    fn classify_bzimage_setup_deeper_need_cmdline_and_init_size() {
        let need = linux_realmode_bytes(1);
        let mut no_cmd = synthetic_linux_boot_protocol_header(1, 0x0202, 0, 0);
        no_cmd.resize(need.max(0x22C), 0);
        match classify_bzimage_setup_deeper(&no_cmd) {
            BzImageEarlyClass::SetupLoadable { next, .. } => {
                assert_eq!(next, BzImageNextStep::NeedCmdlinePtr);
            }
            other => panic!("unexpected {other}"),
        }

        let mut high = synthetic_linux_boot_protocol_header(1, 0x020A, 0x01, 0x0010_0000);
        high.resize(need.max(0x264), 0);
        // init_size @ 0x260 left zero
        match classify_bzimage_setup_deeper(&high) {
            BzImageEarlyClass::SetupLoadable { next, .. } => {
                assert_eq!(next, BzImageNextStep::NeedInitSize);
            }
            other => panic!("unexpected {other}"),
        }

        // With init_size set, fall through to load-high.
        high[0x260..0x264].copy_from_slice(&0x0010_0000u32.to_le_bytes());
        match classify_bzimage_setup_deeper(&high) {
            BzImageEarlyClass::SetupLoadable { next, .. } => {
                assert_eq!(
                    next,
                    BzImageNextStep::LoadHighProtectedKernel {
                        code32_start: 0x0010_0000
                    }
                );
            }
            other => panic!("unexpected {other}"),
        }
    }

    /// Spec: El Torito 1.0 — classify attached no-emul CD as boot candidate.
    #[test]
    fn classify_eltorito_media_boot_no_emul_candidate() {
        let mut m = Machine::new(64 * 1024);
        assert_eq!(
            classify_eltorito_media_boot(&m),
            ElToritoMediaBootClass::NoMedium
        );
        m.attach_atapi_cdrom_image(synthetic_eltorito_hlt_iso());
        match classify_eltorito_media_boot(&m) {
            ElToritoMediaBootClass::NoEmulCandidate {
                load_rba,
                sector_count,
                ..
            } => {
                assert_eq!(load_rba, 24);
                assert_eq!(sector_count, 4);
            }
            other => panic!("unexpected {other}"),
        }
        assert!(classify_eltorito_media_boot(&m).is_boot_candidate());
    }

    /// R14: Linux media classify deepens El Torito + optional bzImage.
    #[test]
    fn classify_linux_media_boot_eltorito_and_bzimage() {
        let mut m = Machine::new(64 * 1024);
        assert_eq!(
            classify_linux_media_boot(&m, None),
            LinuxMediaBootClass::NoMedia
        );
        m.attach_atapi_cdrom_image(synthetic_eltorito_linux_hlt_iso());
        match classify_linux_media_boot(&m, None) {
            LinuxMediaBootClass::ElTorito(e) => assert!(e.is_boot_candidate()),
            other => panic!("unexpected {other}"),
        }
        let need = linux_realmode_bytes(1);
        let mut bz = synthetic_linux_boot_protocol_header(1, 0x0202, 0, 0);
        bz.resize(need.max(0x22C), 0);
        match classify_linux_media_boot(&m, Some(&bz)) {
            LinuxMediaBootClass::ElToritoPlusBzImage { eltorito, bzimage } => {
                assert!(eltorito.is_boot_candidate());
                assert!(matches!(
                    bzimage,
                    BzImageEarlyClass::SetupLoadable {
                        next: BzImageNextStep::NeedCmdlinePtr,
                        ..
                    }
                ));
            }
            other => panic!("unexpected {other}"),
        }
        assert!(classify_linux_media_boot(&m, Some(&bz)).is_boot_candidate());
    }

    /// R14: measure El Torito media — first-failure is synthetic-halt (not Linux shell).
    #[test]
    fn measure_linux_with_eltorito_media_first_failure() {
        let mut m = Machine::new(64 * 1024);
        let report = m
            .measure_linux_with_eltorito_media(64)
            .expect("linux-eltorito");
        assert_eq!(report.kind, GuestOsMeasureKind::LinuxSerialPath);
        assert_eq!(report.first_failure, GuestFirstFailureClass::SyntheticHalt);
        assert_eq!(report.measure.media, GuestBootMedia::ElTorito);
        assert!(report.measure.com1.contains('L') || report.measure.com1 == "LX");
        let text = report.to_string();
        assert!(text.contains("does NOT claim Linux"));
        assert!(!text.contains("serial shell reached"));
    }

    /// R15: arm setup entry at +0x200 (boot.rst).
    #[test]
    fn arm_bzimage_realmode_entry_sets_cs_ip() {
        let mut m = Machine::new(1024 * 1024);
        m.arm_bzimage_realmode_entry(LINUX_REALMODE_LOAD_ADDR)
            .expect("arm");
        assert_eq!(m.cpu.cs.selector, 0x9000);
        assert_eq!(m.cpu.ip16(), 0x0200);
    }

    /// R15: load+arm+measure reaches setup-executed-missing-protected-kernel.
    #[test]
    fn measure_linux_bzimage_setup_entry_next_gap() {
        let mut m = Machine::new(1024 * 1024);
        let (report, gap) = m
            .measure_linux_bzimage_setup_entry(64)
            .expect("bzimage-setup");
        assert_eq!(report.kind, GuestOsMeasureKind::LinuxSerialPath);
        assert_eq!(report.first_failure, GuestFirstFailureClass::SyntheticHalt);
        assert_eq!(report.measure.media, GuestBootMedia::BzImageSetup);
        assert_eq!(gap, LinuxNextGap::SetupExecutedMissingProtectedKernel);
        assert_eq!(gap.tag(), "setup-executed-missing-protected-kernel");
        assert!(report.measure.com1.contains('L') || report.measure.com1 == "LX");
        let text = report.to_string();
        assert!(text.contains("does NOT claim Linux"));
        assert!(!text.contains("serial shell reached"));
    }

    /// Spec: setup loadable without arm → setup-loaded-missing-entry.
    #[test]
    fn classify_linux_next_gap_setup_loaded_missing_entry() {
        let need = linux_realmode_bytes(1);
        let mut bz = synthetic_linux_boot_protocol_header(1, 0x0200, 0, 0);
        bz.resize(need, 0);
        let media = LinuxMediaBootClass::BzImage(classify_bzimage_early(&bz));
        let gap = classify_linux_next_gap(&GuestFirstFailureClass::SyntheticHalt, &media, false);
        assert_eq!(gap, LinuxNextGap::SetupLoadedMissingEntry);
    }

    /// R15: El Torito catalog→payload peeks HLT stub vs bzImage setup.
    #[test]
    fn classify_eltorito_boot_payload_hlt_vs_bzimage() {
        let iso = synthetic_eltorito_bzimage_iso();
        let boot = &iso[24 * firmware_interface::EL_TORITO_SECTOR_BYTES
            ..24 * firmware_interface::EL_TORITO_SECTOR_BYTES + 2048];
        let raw = classify_bzimage_setup_deeper(boot);
        assert!(
            matches!(raw, BzImageEarlyClass::SetupLoadable { .. }),
            "raw boot bytes should be setup-loadable, got {raw}"
        );

        let mut m = Machine::new(64 * 1024);
        assert!(matches!(
            classify_eltorito_boot_payload(&m),
            ElToritoPayloadClass::Media(ElToritoMediaBootClass::NoMedium)
        ));
        m.attach_atapi_cdrom_image(synthetic_eltorito_linux_hlt_iso());
        match classify_eltorito_boot_payload(&m) {
            ElToritoPayloadClass::NoEmulHltStub { .. } => {}
            other => panic!("expected hlt stub, got {other}"),
        }
        assert!(!classify_eltorito_boot_payload(&m).is_bzimage_candidate());

        m.attach_atapi_cdrom_image(iso);
        match classify_eltorito_boot_payload(&m) {
            ElToritoPayloadClass::NoEmulBzImage { bzimage, .. } => {
                assert!(matches!(bzimage, BzImageEarlyClass::SetupLoadable { .. }));
            }
            other => panic!("expected bzImage payload, got {other}"),
        }
        assert!(classify_eltorito_boot_payload(&m).is_bzimage_candidate());
    }
}
