//! Guest disk boot measure-first harness (FreeDOS/Linux path prep).
//!
//! Loads a boot sector to `0x7C00` then reuses [`Machine::probe_post`] to record
//! the **first** stop reason. This does **not** claim guest boot success.
//!
//! Spec: IBM PC BIOS INT 19h handoff + existing POST probe diagnostics.

use crate::post_probe::PostReport;
use crate::{Machine, MachineError};

/// Which host boot-sector helper to use before measuring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestBootMedia {
    /// [`Machine::load_mbr_to_7c00`] — IDE LBA0 prefer, else floppy.
    IdePrefer,
    /// [`Machine::load_floppy_boot_to_7c00`] — floppy CHS `(0,0,1)` only.
    FloppyFirst,
}

/// Result of preparing media + measuring first failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestBootMeasure {
    /// How the boot sector was selected.
    pub media: GuestBootMedia,
    /// Probe report (first failure / halt / budget). **Not** a success claim.
    pub report: PostReport,
}

impl std::fmt::Display for GuestBootMeasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "guest-measure: media={} (measure-first; not a boot-success claim)",
            match self.media {
                GuestBootMedia::IdePrefer => "ide-prefer",
                GuestBootMedia::FloppyFirst => "floppy-first",
            }
        )?;
        write!(f, "{}", self.report)
    }
}

impl Machine {
    /// Load boot sector per `media`, then [`Self::probe_post`] under `max_steps`.
    ///
    /// Returns [`MachineError`] only for media/signature/RAM problems before
    /// execution starts. Execution stops are always captured inside the report.
    pub fn measure_guest_boot(
        &mut self,
        media: GuestBootMedia,
        max_steps: u64,
    ) -> Result<GuestBootMeasure, MachineError> {
        match media {
            GuestBootMedia::IdePrefer => self.load_mbr_to_7c00()?,
            GuestBootMedia::FloppyFirst => self.load_floppy_boot_to_7c00()?,
        }
        let report = self.probe_post(max_steps);
        Ok(GuestBootMeasure { media, report })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbr::{MBR_SECTOR_SIZE, MBR_SIGNATURE_HI, MBR_SIGNATURE_LO};
    use crate::post_probe::{PostFailureKind, PostStopReason};
    use devices::FDC_1440_IMAGE_SIZE;

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

    /// Measure-first: HLT boot sector stops with an honest halt reason.
    #[test]
    fn measure_guest_boot_hlt_records_halt() {
        let mut m = Machine::with_ide(64 * 1024, synthetic_mbr_hlt());
        let measure = m
            .measure_guest_boot(GuestBootMedia::IdePrefer, 64)
            .expect("measure");
        assert!(matches!(measure.report.stop, PostStopReason::Halted));
        let text = measure.to_string();
        assert!(text.contains("guest-measure:"));
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
}
