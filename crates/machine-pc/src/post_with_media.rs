//! SeaBIOS POST measure with INT 19h-candidate boot media attached.
//!
//! After CF9, no-media POST reboot-loops around `F000:9842` (`boot_fail` →
//! `qemu_reboot`). This harness attaches [`crate::boot_media`] synthetic HD
//! media and re-runs [`Machine::probe_post`] under the documented ~20M budget.
//!
//! Honesty: recording a stop CS:IP is **not** a claim that SeaBIOS INT 19h
//! completed or that a guest OS booted. See `docs/boot-r14-post-with-media.md`.
//!
//! Spec: IBM PC BIOS INT 19h / OSDev Boot Sequence; `docs/post-c897-remeasure.md`.

use crate::boot_media::Int19BootMediaClass;
use crate::post_probe::{PostReport, PostStopReason};
use crate::{Machine, MachineError};

/// Documented instruction budget for POST-with-media remeasure (same class as
/// the no-media CF9 remeasure in `docs/post-c897-remeasure.md`).
pub const POST_WITH_MEDIA_BUDGET_STEPS: u64 = 20_000_000;

/// SeaBIOS no-media reboot-loop class CS (real-mode).
pub const NO_MEDIA_REBOOT_CLASS_CS: u16 = 0xF000;
/// SeaBIOS no-media reboot-loop class IP (`boot_fail` / yield poll site).
pub const NO_MEDIA_REBOOT_CLASS_IP: u16 = 0x9842;

/// How a POST-with-media stop relates to the no-media `F000:9842` class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostWithMediaClass {
    /// Still at the documented no-media reboot-loop CS:IP.
    StillNoMediaRebootClass,
    /// Guest halted (e.g. synthetic HLT MBR at `0000:7C00`) — past reboot loop.
    GuestHaltedAtBootSector,
    /// Stopped elsewhere (budget / failure) — past or off the no-media class.
    OtherStop { cs: u16, eip: u32 },
}

impl PostWithMediaClass {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::StillNoMediaRebootClass => "still-no-media-reboot-class",
            Self::GuestHaltedAtBootSector => "guest-halted-at-boot-sector",
            Self::OtherStop { .. } => "other-stop",
        }
    }

    /// True when the stop is no longer the no-media `F000:9842` class.
    pub fn past_no_media_reboot_class(&self) -> bool {
        !matches!(self, Self::StillNoMediaRebootClass)
    }
}

impl std::fmt::Display for PostWithMediaClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OtherStop { cs, eip } => {
                write!(f, "other-stop:{cs:04X}:{eip:08X}")
            }
            other => f.write_str(other.tag()),
        }
    }
}

/// Classify a POST stop relative to the documented no-media reboot loop.
pub fn classify_post_with_media_stop(report: &PostReport) -> PostWithMediaClass {
    let cs = report.stop_site.cs;
    let eip = report.stop_site.eip;
    let ip16 = (eip & 0xFFFF) as u16;
    if cs == NO_MEDIA_REBOOT_CLASS_CS && ip16 == NO_MEDIA_REBOOT_CLASS_IP {
        return PostWithMediaClass::StillNoMediaRebootClass;
    }
    // HLT advances IP past the opcode; accept `0000:7C00`..`7C10` as boot-sector halt.
    if matches!(report.stop, PostStopReason::Halted) && cs == 0 && (0x7C00..=0x7C10).contains(&ip16)
    {
        return PostWithMediaClass::GuestHaltedAtBootSector;
    }
    PostWithMediaClass::OtherStop { cs, eip }
}

/// POST-with-media measure report (diagnostic; not boot success).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostWithMediaReport {
    /// Instruction budget requested.
    pub budget: u64,
    /// Attached INT 19h media classify tag.
    pub media: Int19BootMediaClass,
    /// Whether media is an INT 19h candidate.
    pub media_is_int19_candidate: bool,
    /// Stop class vs no-media reboot loop.
    pub class: PostWithMediaClass,
    /// Underlying POST probe report.
    pub report: PostReport,
    /// Explicit non-claim.
    pub honesty: &'static str,
}

impl std::fmt::Display for PostWithMediaReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "post-with-media: budget={} media={} int19-candidate={} class={} (NOT SeaBIOS INT19 success / NOT OS boot)",
            self.budget,
            self.media,
            self.media_is_int19_candidate,
            self.class
        )?;
        writeln!(f, "  honesty: {}", self.honesty)?;
        write!(f, "{}", self.report)
    }
}

impl Machine {
    /// Attach INT 19h-candidate HD (if needed) and run [`Self::probe_post`].
    ///
    /// Caller must already map SeaBIOS via [`Self::with_bios_rom`]. Uses
    /// [`crate::boot_media::synthetic_int19_bootable_hd`] when no IDE image is
    /// present.
    ///
    /// Does **not** claim SeaBIOS INT 19h or guest OS boot success.
    pub fn measure_post_with_bootable_hd(
        &mut self,
        max_steps: u64,
    ) -> Result<PostWithMediaReport, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_bootable_hd_for_int19();
        }
        let media = self.classify_attached_ide_int19();
        let report = self.probe_post(max_steps);
        let class = classify_post_with_media_stop(&report);
        Ok(PostWithMediaReport {
            budget: max_steps,
            media,
            media_is_int19_candidate: media.is_int19_candidate(),
            class,
            report,
            honesty: "POST-with-media diagnostic only — does NOT claim SeaBIOS INT 19h success, FreeDOS, or Linux boot.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_media::Int19BootMediaClass;
    use crate::post_probe::seabios_image_path;
    use crate::post_probe::POST_OPCODE_WINDOW_LEN;
    use crate::post_spin::PostPcSite;

    fn site(cs: u16, eip: u32) -> PostPcSite {
        PostPcSite {
            cs,
            eip,
            cs_default_big: false,
            linear_pc: u64::from(cs) * 16 + u64::from(eip),
        }
    }

    fn report_at(stop: PostStopReason, cs: u16, eip: u32) -> PostReport {
        PostReport {
            steps: 1,
            idle_steps: 0,
            stop,
            stop_bytes: [None; POST_OPCODE_WINDOW_LEN],
            stop_site: site(cs, eip),
            spin: None,
            unclaimed_ports: vec![],
            unclaimed_port_overflow: false,
            unmapped_mmio: vec![],
            unmapped_mmio_overflow: false,
            post_codes: vec![],
            last_post_code: None,
            post_code_overflow: false,
            com1: String::new(),
            debug: String::new(),
        }
    }

    #[test]
    fn classify_still_no_media_reboot_class() {
        let r = report_at(PostStopReason::StepBudgetExhausted, 0xF000, 0x9842);
        assert_eq!(
            classify_post_with_media_stop(&r),
            PostWithMediaClass::StillNoMediaRebootClass
        );
        assert!(!classify_post_with_media_stop(&r).past_no_media_reboot_class());
    }

    #[test]
    fn classify_guest_halted_at_7c00() {
        let r = report_at(PostStopReason::Halted, 0x0000, 0x7C01);
        assert_eq!(
            classify_post_with_media_stop(&r),
            PostWithMediaClass::GuestHaltedAtBootSector
        );
        assert!(classify_post_with_media_stop(&r).past_no_media_reboot_class());
    }

    #[test]
    fn classify_other_stop() {
        let r = report_at(PostStopReason::StepBudgetExhausted, 0xF000, 0xC897);
        match classify_post_with_media_stop(&r) {
            PostWithMediaClass::OtherStop { cs, eip } => {
                assert_eq!(cs, 0xF000);
                assert_eq!(eip, 0xC897);
            }
            other => panic!("unexpected {other}"),
        }
    }

    /// Harness: attach INT19 HD on a BIOS-mapped machine and measure (skip if no SeaBIOS).
    #[test]
    fn measure_post_with_bootable_hd_harness() {
        let Some(path) = seabios_image_path() else {
            eprintln!("skipping: no SeaBIOS image");
            return;
        };
        let image = std::fs::read(&path).expect("read SeaBIOS");
        let mut m = Machine::with_bios_rom(32 * 1024 * 1024, &image).expect("map");
        // Short budget validates attach + classify; full 20M via X86WASM_POST_MEDIA_FULL=1.
        let budget = if std::env::var_os("X86WASM_POST_MEDIA_FULL").is_some() {
            POST_WITH_MEDIA_BUDGET_STEPS
        } else {
            64_000
        };
        let report = m
            .measure_post_with_bootable_hd(budget)
            .expect("post-with-media");
        assert!(report.media_is_int19_candidate);
        assert!(matches!(
            report.media,
            Int19BootMediaClass::HdActivePartition { .. }
        ));
        assert!(report.report.steps > 0 || report.report.idle_steps > 0);
        eprintln!("{report}");
    }
}
