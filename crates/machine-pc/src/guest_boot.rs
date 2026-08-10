//! Guest disk boot measure harness v2 (FreeDOS/Linux serial-path prep).
//!
//! Loads a boot sector / El Torito image, then reuses [`Machine::probe_post`] to
//! record the **first** stop reason plus serial capture and named checkpoints.
//! This does **not** claim FreeDOS, Linux, or Milestone 2 boot success.
//!
//! Spec: IBM PC BIOS INT 19h handoff + El Torito 1.0 no-emul load + existing
//! POST probe diagnostics (`docs/boot-r8-guest-measure-v2.md`).

use crate::post_probe::{PostReport, PostStopReason};
use crate::{Machine, MachineError};

/// Harness schema version for CLI/report consumers.
pub const GUEST_BOOT_MEASURE_VERSION: u32 = 2;

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
    /// Probe report (first failure / halt / budget). **Not** a success claim.
    pub report: PostReport,
}

impl GuestBootMeasure {
    /// True when any serial/debug byte was captured (does not imply boot success).
    pub fn serial_captured(&self) -> bool {
        !self.com1.is_empty() || !self.debug.is_empty()
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
        write!(f, "{}", self.report)
    }
}

impl Machine {
    /// Load boot image per `media`, then [`Self::probe_post`] under `max_steps`.
    ///
    /// Returns [`MachineError`] only for media/signature/RAM problems before
    /// execution starts. Execution stops are always captured inside the report.
    ///
    /// v2 also records checkpoints and copies COM1/debug serial at the stop.
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
        checkpoints.push(GuestBootCheckpoint::StopRecorded);
        Ok(GuestBootMeasure {
            version: GUEST_BOOT_MEASURE_VERSION,
            media,
            checkpoints,
            com1,
            debug,
            report,
        })
    }
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
}
