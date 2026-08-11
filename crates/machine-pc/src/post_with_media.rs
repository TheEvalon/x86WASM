//! SeaBIOS POST measure with INT 19h-candidate boot media attached.
//!
//! After CF9, no-media POST reboot-loops around `F000:9842` (`boot_fail` →
//! `qemu_reboot`). With media, the documented 20M-step stop lands at
//! `F000:C897` (SeaBIOS `wait_irq` yield) — see R14
//! `docs/boot-r14-post-with-media.md` and R15 `docs/post-r15-c897-with-media.md`.
//!
//! Honesty: recording a stop CS:IP / idle ratio / CF9 pulse count is **not** a
//! claim that SeaBIOS POST completed or that INT 19h / a guest OS booted.
//!
//! Spec: IBM PC BIOS INT 19h / OSDev Boot Sequence; `docs/post-c897-*.md`.

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

/// SeaBIOS late-POST `wait_irq` yield CS (real-mode).
///
/// Spec evidence: ROM at `F000:C895` is `sti; hlt; cli; cld; ret` — stop IP
/// `C897` is the `cli` after HLT when the budget ends mid-yield
/// (`docs/post-c897-cf9-diagnosis.md`).
pub const WAIT_IRQ_CLASS_CS: u16 = 0xF000;
/// SeaBIOS late-POST `wait_irq` yield IP (`cli` after HLT).
pub const WAIT_IRQ_CLASS_IP: u16 = 0xC897;

/// Idle share threshold: `idle_steps / (steps + idle_steps)` ≥ this → idle-dominant.
pub const POST_MEDIA_IDLE_DOMINANT_PCT: u32 = 35;

/// Busy vs idle dominance at a POST-with-media stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostMediaActivity {
    /// Halt-idle quanta dominate the budget (typical `wait_irq` yield sampling).
    IdleDominant { idle_pct: u32 },
    /// Retired instructions dominate (busy spin / work between yields).
    BusyDominant { idle_pct: u32 },
}

impl PostMediaActivity {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::IdleDominant { .. } => "idle-dominant",
            Self::BusyDominant { .. } => "busy-dominant",
        }
    }

    pub fn idle_pct(self) -> u32 {
        match self {
            Self::IdleDominant { idle_pct } | Self::BusyDominant { idle_pct } => idle_pct,
        }
    }
}

impl std::fmt::Display for PostMediaActivity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IdleDominant { idle_pct } => write!(f, "idle-dominant:{idle_pct}%"),
            Self::BusyDominant { idle_pct } => write!(f, "busy-dominant:{idle_pct}%"),
        }
    }
}

/// Whether ICH CF9 system-reset pulses were observed during the measure.
///
/// Spec: ICH Reset Control at `CF9h`; SeaBIOS `qemu_reboot` writes RST_CPU.
/// Pulses imply a completed `boot_fail`→reboot path; absence at `C897` is still
/// consistent with mid-POST `wait_irq` (not a reboot claim either way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostMediaRebootSignal {
    /// No CF9 RST_CPU pulse counted — stop is wait/yield class, not reboot-loop proof.
    WaitIrqNoRebootYet,
    /// At least one CF9 pulse — firmware reached a reboot attempt during the budget.
    RebootPulseSeen { pulses: u64 },
}

impl PostMediaRebootSignal {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::WaitIrqNoRebootYet => "wait-irq-no-reboot-yet",
            Self::RebootPulseSeen { .. } => "reboot-pulse-seen",
        }
    }
}

impl std::fmt::Display for PostMediaRebootSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitIrqNoRebootYet => f.write_str(self.tag()),
            Self::RebootPulseSeen { pulses } => write!(f, "reboot-pulse-seen:{pulses}"),
        }
    }
}

/// How a POST-with-media stop relates to the no-media `F000:9842` / `C897` classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostWithMediaClass {
    /// Still at the documented no-media reboot-loop CS:IP.
    StillNoMediaRebootClass,
    /// Guest halted (e.g. synthetic HLT MBR at `0000:7C00`) — past reboot loop.
    GuestHaltedAtBootSector,
    /// Stopped at documented late-POST `wait_irq` (`F000:C897`) with media.
    ///
    /// **Not** POST-complete. Carries idle/busy + CF9 reboot signal for triage.
    WaitIrqYield {
        activity: PostMediaActivity,
        reboot: PostMediaRebootSignal,
    },
    /// Stopped elsewhere (budget / failure) — past or off the no-media class.
    OtherStop { cs: u16, eip: u32 },
}

impl PostWithMediaClass {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::StillNoMediaRebootClass => "still-no-media-reboot-class",
            Self::GuestHaltedAtBootSector => "guest-halted-at-boot-sector",
            Self::WaitIrqYield { .. } => "wait-irq-yield",
            Self::OtherStop { .. } => "other-stop",
        }
    }

    /// True when the stop is no longer the no-media `F000:9842` class.
    pub fn past_no_media_reboot_class(&self) -> bool {
        !matches!(self, Self::StillNoMediaRebootClass)
    }

    /// True when classified as the documented `F000:C897` wait_irq yield site.
    pub fn is_wait_irq_yield(&self) -> bool {
        matches!(self, Self::WaitIrqYield { .. })
    }
}

impl std::fmt::Display for PostWithMediaClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitIrqYield { activity, reboot } => {
                write!(f, "wait-irq-yield:F000:C897/{activity}/{reboot}")
            }
            Self::OtherStop { cs, eip } => {
                write!(f, "other-stop:{cs:04X}:{eip:08X}")
            }
            other => f.write_str(other.tag()),
        }
    }
}

/// Idle percentage from a POST report (`0..=100`).
pub fn post_report_idle_pct(report: &PostReport) -> u32 {
    let total = report.steps.saturating_add(report.idle_steps);
    if total == 0 {
        return 0;
    }
    ((report.idle_steps.saturating_mul(100)) / total) as u32
}

/// Classify busy vs idle from a POST report.
pub fn classify_post_media_activity(report: &PostReport) -> PostMediaActivity {
    let idle_pct = post_report_idle_pct(report);
    if idle_pct >= POST_MEDIA_IDLE_DOMINANT_PCT {
        PostMediaActivity::IdleDominant { idle_pct }
    } else {
        PostMediaActivity::BusyDominant { idle_pct }
    }
}

/// Classify CF9 reboot signal from pulse count.
pub fn classify_post_media_reboot(cf9_pulses: u64) -> PostMediaRebootSignal {
    if cf9_pulses == 0 {
        PostMediaRebootSignal::WaitIrqNoRebootYet
    } else {
        PostMediaRebootSignal::RebootPulseSeen { pulses: cf9_pulses }
    }
}

/// Classify a POST stop relative to the documented no-media / wait_irq classes.
///
/// `cf9_pulses` is [`devices::Cf9Reset::reset_pulse_count`] after the probe.
/// Does **not** claim POST complete.
pub fn classify_post_with_media_stop(report: &PostReport, cf9_pulses: u64) -> PostWithMediaClass {
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
    if cs == WAIT_IRQ_CLASS_CS && ip16 == WAIT_IRQ_CLASS_IP {
        return PostWithMediaClass::WaitIrqYield {
            activity: classify_post_media_activity(report),
            reboot: classify_post_media_reboot(cf9_pulses),
        };
    }
    PostWithMediaClass::OtherStop { cs, eip }
}

/// POST-with-media measure report (diagnostic; not boot success / not POST complete).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostWithMediaReport {
    /// Instruction budget requested.
    pub budget: u64,
    /// Attached INT 19h media classify tag.
    pub media: Int19BootMediaClass,
    /// Whether media is an INT 19h candidate.
    pub media_is_int19_candidate: bool,
    /// CF9 RST_CPU pulse count observed after the probe.
    pub cf9_pulses: u64,
    /// Stop class vs no-media / wait_irq.
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
            "post-with-media: budget={} media={} int19-candidate={} cf9-pulses={} class={} (NOT POST complete / NOT SeaBIOS INT19 success / NOT OS boot)",
            self.budget,
            self.media,
            self.media_is_int19_candidate,
            self.cf9_pulses,
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
    /// Does **not** claim SeaBIOS INT 19h, POST complete, or guest OS boot.
    pub fn measure_post_with_bootable_hd(
        &mut self,
        max_steps: u64,
    ) -> Result<PostWithMediaReport, MachineError> {
        if !self.ide.present || self.ide.image.is_empty() {
            self.attach_bootable_hd_for_int19();
        }
        let media = self.classify_attached_ide_int19();
        let report = self.probe_post(max_steps);
        let cf9_pulses = self.cf9.reset_pulse_count();
        let class = classify_post_with_media_stop(&report, cf9_pulses);
        Ok(PostWithMediaReport {
            budget: max_steps,
            media,
            media_is_int19_candidate: media.is_int19_candidate(),
            cf9_pulses,
            class,
            report,
            honesty: "POST-with-media diagnostic only — does NOT claim POST complete, SeaBIOS INT 19h success, FreeDOS, or Linux boot.",
        })
    }

    /// Host INT 19h-order load + short probe toward `0000:7C00` halt class (R15).
    ///
    /// Uses [`Self::host_int19_load_boot_sector`] then [`Self::probe_post`]. When
    /// media is a synthetic HLT sector, classifies as
    /// [`PostWithMediaClass::GuestHaltedAtBootSector`].
    ///
    /// Honesty: this is a **host** path that demonstrates the boot-sector
    /// execution classify — it does **not** claim SeaBIOS INT 19h completed
    /// (POST-with-media still stops at `F000:C897`; see
    /// `docs/post-r15-c897-with-media.md`, `docs/boot-r15-int19-handoff.md`).
    pub fn measure_host_int19_boot_sector(
        &mut self,
        chain_active_vbr: bool,
        max_steps: u64,
    ) -> Result<Int19HandoffReport, MachineError> {
        if (!self.ide.present || self.ide.image.is_empty())
            && self.fdc.read_sector(0, 0, 1).is_none()
        {
            self.attach_bootable_hd_for_int19();
        }
        let media = self.host_int19_load_boot_sector(chain_active_vbr)?;
        let report = self.probe_post(max_steps);
        let cf9_pulses = self.cf9.reset_pulse_count();
        let class = classify_post_with_media_stop(&report, cf9_pulses);
        Ok(Int19HandoffReport {
            media,
            class,
            report,
            honesty: "Host INT19-order handoff measure — does NOT claim SeaBIOS INT 19h success or OS boot.",
        })
    }
}

/// Host INT 19h-order handoff measure (R15; not SeaBIOS INT19 success).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Int19HandoffReport {
    /// Which media the host helper loaded.
    pub media: crate::mbr::Int19HandoffMedia,
    /// Stop class (expect [`PostWithMediaClass::GuestHaltedAtBootSector`] on HLT fixtures).
    pub class: PostWithMediaClass,
    /// Underlying probe report.
    pub report: PostReport,
    /// Explicit non-claim.
    pub honesty: &'static str,
}

impl std::fmt::Display for Int19HandoffReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "int19-handoff: media={} class={} (NOT SeaBIOS INT19 success / NOT OS boot)",
            self.media, self.class
        )?;
        writeln!(f, "  honesty: {}", self.honesty)?;
        write!(f, "{}", self.report)
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
        report_budget(stop, cs, eip, 1000, 0)
    }

    fn report_budget(
        stop: PostStopReason,
        cs: u16,
        eip: u32,
        steps: u64,
        idle_steps: u64,
    ) -> PostReport {
        PostReport {
            steps,
            idle_steps,
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
            classify_post_with_media_stop(&r, 0),
            PostWithMediaClass::StillNoMediaRebootClass
        );
        assert!(!classify_post_with_media_stop(&r, 0).past_no_media_reboot_class());
    }

    #[test]
    fn classify_guest_halted_at_7c00() {
        let r = report_at(PostStopReason::Halted, 0x0000, 0x7C01);
        assert_eq!(
            classify_post_with_media_stop(&r, 0),
            PostWithMediaClass::GuestHaltedAtBootSector
        );
        assert!(classify_post_with_media_stop(&r, 0).past_no_media_reboot_class());
    }

    /// R15: `F000:C897` is wait_irq yield — idle-dominant, no CF9 pulse yet.
    #[test]
    fn classify_c897_wait_irq_idle_no_reboot() {
        // ~40% idle matches R14 measured post-with-media halt-idle.
        let r = report_budget(
            PostStopReason::StepBudgetExhausted,
            0xF000,
            0xC897,
            600,
            400,
        );
        match classify_post_with_media_stop(&r, 0) {
            PostWithMediaClass::WaitIrqYield { activity, reboot } => {
                assert!(matches!(activity, PostMediaActivity::IdleDominant { .. }));
                assert_eq!(activity.idle_pct(), 40);
                assert_eq!(reboot, PostMediaRebootSignal::WaitIrqNoRebootYet);
            }
            other => panic!("unexpected {other}"),
        }
        assert!(classify_post_with_media_stop(&r, 0).is_wait_irq_yield());
        assert!(classify_post_with_media_stop(&r, 0).past_no_media_reboot_class());
    }

    /// R15: busy-dominant C897 + CF9 pulses → reboot signal (still not POST complete).
    #[test]
    fn classify_c897_wait_irq_busy_with_reboot_pulse() {
        let r = report_budget(
            PostStopReason::StepBudgetExhausted,
            0xF000,
            0xC897,
            900,
            100,
        );
        match classify_post_with_media_stop(&r, 3) {
            PostWithMediaClass::WaitIrqYield { activity, reboot } => {
                assert!(matches!(activity, PostMediaActivity::BusyDominant { .. }));
                assert_eq!(activity.idle_pct(), 10);
                assert_eq!(reboot, PostMediaRebootSignal::RebootPulseSeen { pulses: 3 });
            }
            other => panic!("unexpected {other}"),
        }
        let text = format!("{}", classify_post_with_media_stop(&r, 3));
        assert!(text.contains("wait-irq-yield"));
        assert!(text.contains("busy-dominant"));
        assert!(text.contains("reboot-pulse-seen:3"));
    }

    #[test]
    fn classify_other_stop_not_c897() {
        let r = report_at(PostStopReason::StepBudgetExhausted, 0xF000, 0xABCD);
        match classify_post_with_media_stop(&r, 0) {
            PostWithMediaClass::OtherStop { cs, eip } => {
                assert_eq!(cs, 0xF000);
                assert_eq!(eip, 0xABCD);
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
        // Honesty: harness records class only — does not assert POST complete.
        let _ = report.class.is_wait_irq_yield();
        eprintln!("{report}");
    }

    /// R15: host INT19 handoff reaches guest-halted-at-boot-sector (not SeaBIOS POST).
    #[test]
    fn measure_host_int19_boot_sector_halts_at_7c00() {
        let mut m = Machine::new(64 * 1024);
        m.attach_bootable_hd_for_int19();
        let report = m
            .measure_host_int19_boot_sector(false, 64)
            .expect("int19-handoff");
        assert_eq!(report.media.tag(), "hd-mbr");
        assert_eq!(report.class, PostWithMediaClass::GuestHaltedAtBootSector);
        assert!(matches!(report.report.stop, PostStopReason::Halted));
        let text = report.to_string();
        assert!(text.contains("guest-halted-at-boot-sector"));
        assert!(text.contains("NOT SeaBIOS INT19"));
    }

    /// R15: host INT19 + VBR chain also classifies 7C00 halt.
    #[test]
    fn measure_host_int19_vbr_chain_halts_at_7c00() {
        let mut m = Machine::new(64 * 1024);
        m.attach_bootable_hd_for_int19();
        let report = m
            .measure_host_int19_boot_sector(true, 64)
            .expect("int19-vbr");
        assert_eq!(report.media.tag(), "hd-active-vbr");
        assert_eq!(report.class, PostWithMediaClass::GuestHaltedAtBootSector);
    }
}
