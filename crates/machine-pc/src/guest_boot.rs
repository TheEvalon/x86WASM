//! Guest disk boot measure harness v2/v4 (FreeDOS/Linux serial-path prep).
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
//! `docs/boot-r10-linux-serial-first-failure.md`).

use crate::post_probe::{PostFailureKind, PostReport, PostStopReason};
use crate::{Machine, MachineError};

/// Harness schema version for CLI/report consumers (v2 checkpoints + serial).
pub const GUEST_BOOT_MEASURE_VERSION: u32 = 2;

/// FreeDOS-like / Linux-serial measure report schema (v4 = first-failure class).
pub const GUEST_OS_MEASURE_VERSION: u32 = 4;

/// Which host boot helper to use before measuring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestBootMedia {
    /// [`Machine::load_mbr_to_7c00`] — IDE LBA0 prefer, else floppy.
    IdePrefer,
    /// [`Machine::load_floppy_boot_to_7c00`] — floppy CHS `(0,0,1)` only.
    FloppyFirst,
    /// [`Machine::load_eltorito_to_7c00`] — no-emulation CD boot image.
    ElTorito,
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
pub fn classify_guest_first_failure(report: &PostReport) -> GuestFirstFailureClass {
    match &report.stop {
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
    }
}

/// v4 OS-path measure: wraps v2 plus honesty / first-failure class / gap list.
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
    /// Explicit non-claim sentence for reports/CLI.
    pub honesty: &'static str,
    /// Remaining gaps toward a real guest (not M2 exit), including class note.
    pub gaps: Vec<&'static str>,
}

impl std::fmt::Display for GuestOsMeasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            GuestOsMeasureKind::FreeDosLike => "freedos-like",
            GuestOsMeasureKind::LinuxSerialPath => "linux-serial-path",
        };
        writeln!(
            f,
            "guest-os-measure-v{}: kind={} first-failure={} bucket={} site={} (NOT an OS boot / NOT Milestone 2 exit)",
            self.version, kind, self.first_failure, self.failure_bucket, self.failure_site
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
        write!(f, "{}", self.measure)
    }
}

impl Machine {
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
    ) -> GuestOsMeasure {
        let int13_probe = self.probe_int13_hd_extensions_status();
        let first_failure = classify_guest_first_failure(&measure.report, Some(&int13_probe));
        let failure_bucket = first_failure.bucket();
        let failure_site = GuestFailureSite {
            cs: measure.report.stop_site.cs,
            eip: measure.report.stop_site.eip,
        };
        gaps.push(first_failure.gap_note());
        GuestOsMeasure {
            version: GUEST_OS_MEASURE_VERSION,
            kind,
            measure,
            first_failure,
            failure_bucket,
            failure_site,
            int13_probe,
            honesty,
            gaps,
        }
    }

    /// Attach the in-tree FreeDOS-*like* IDE fixture (if no IDE yet) and measure.
    ///
    /// The fixture is a signed MBR that prints `FD` to COM1 and a VGA glyph,
    /// then `HLT`, plus a second-sector payload marker. It is **not** FreeDOS
    /// and must never be reported as a FreeDOS prompt.
    pub fn measure_freedos_like(&mut self, max_steps: u64) -> Result<GuestOsMeasure, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_ide_image(synthetic_freedos_like_disk());
        }
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
        ))
    }

    /// Attach a synthetic Linux serial-path stub (if no IDE yet) and measure.
    ///
    /// Captures COM1 from a guest that prints a short banner then `HLT`. Does
    /// **not** load a bzImage, enter protected mode, or claim Linux boot / M2 exit.
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
            ],
        ))
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
        let text = report.to_string();
        assert!(text.contains("NOT an OS boot"));
        assert!(text.contains("does NOT claim a FreeDOS prompt"));
        assert!(text.contains("freedos-like"));
        assert!(text.contains("first-failure=synthetic-halt"));
        assert!(report.gaps.iter().any(|g| g.contains("prompt")));
        assert!(report
            .gaps
            .iter()
            .any(|g| g.contains("not FreeDOS/Linux progress")));
        // Payload marker exists on disk but is not a boot claim.
        assert!(m.ide.image[MBR_SECTOR_SIZE..].starts_with(b"FREEDOS-LIKE-PAYLOAD"));
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
        let text = report.to_string();
        assert!(text.contains("NOT an OS boot"));
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
        let text = report.to_string();
        assert!(text.contains("NOT Milestone 2 exit"));
        assert!(text.contains("linux-serial-path"));
        assert!(text.contains("first-failure=synthetic-halt"));
        assert!(matches!(report.measure.report.stop, PostStopReason::Halted));
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
        assert!(report.gaps.iter().any(|g| g.contains("bzImage")));
        let text = report.to_string();
        assert!(text.contains("NOT Milestone 2 exit"));
        assert!(!text.contains("Linux shell"));
    }
}
