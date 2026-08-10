# Milestone 2 Round 12 parallel integration

Branch: `merge/m2-r12-parallel-16` (base `merge/m2-r11-parallel-16` / `01050d2`).

Five lanes merged in order: **post-c897** → usb-timer → display-fw → boot-guest → cpu-vm86.

## Landed

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| post-c897 | `16d0235` | ICH `0xCF9` reset (`devices::Cf9Reset`); RST_CPU → shared reset pulse. Diagnosed `F000:C897` as SeaBIOS `wait_irq` sampling, not the causal hang — missing CF9 caused `qemu_reboot` INT3. 20M remeasure advances past C897 to `F000:9842` reboot loop (no boot media). | `docs/post-c897-cf9-diagnosis.md`, `docs/post-c897-remeasure.md` |
| usb-timer | `f5eec2c` | UHCI QH horizontal depth 4; USBSTS/USBINTR gating; LAPIC ICR presence/self-IPI; HPET FSB/MSI capability clear + IRQ-route honesty | `docs/uhci-r12-*`, `docs/lapic-r12-icr.md`, `docs/hpet-r12-msi-irq.md` |
| display-fw | `9c24c88` | INT 10h AH=01 cursor type; AH=09/0A write char; BDA columns/page polish; VBE 4F00 deepen **without** LFB | `docs/vga-r12-*`, `docs/firmware-r12-vbe-host-stub.md` |
| boot-guest | `031ae8f` | INT 13h AH=04 VERIFY; AH=41 CX bitmap honesty; FreeDOS next-gap classify v5; Linux bzImage early classify/setup load | `docs/storage-r12-*`, `docs/boot-r12-*` |
| cpu-vm86 | `f7668a9` | `CR4.VME` sticky without `CPUID.VME`; soft-int redirect bitmap stub; 16-bit IDT gate from VM86; INTO/#OF vs INT n under VME | `docs/cpu-r12-*` |

## POST / CF9 note

- **CF9 is in this merge** (`crates/devices/src/cf9.rs`, wired on `MachineBus`).
- Historical stop `F000:C897` was late-POST idle sampling; causal failure after longer budgets was INT3 in `qemu_reboot` without port `0xCF9`.
- After CF9: 2M steps may still report `F000:C897` (budget inside `wait_irq`); **20M steps** stop at `F000:9842` (real-mode reboot/yield loop, no media). See `docs/post-c897-remeasure.md`.
- **SeaBIOS POST still does not complete**; M2 exits (POST complete / FreeDOS prompt / Linux serial shell) remain open.

## Merge notes

- Auto-merged `machine-pc/src/lib.rs`, `devices/src/lib.rs`, `docs/sources.md` across POST/usb/display/boot.
- Manual `plan.md` conflict resolution only (keep all lane status lines).
- CPU lane last: sole writer of `x86-interpreter/src/lib.rs`.
- Boot lane included post-slice clippy `FloppyXfer` enum fix (`031ae8f`).

## Honesty that survives

- No guest LFB / VBE LFB ModeAttributes; `PhysBasePtr` stays 0.
- No `CPUID.VME` / full VME (`CLI`/`STI` VIF path incomplete).
- CPUID APIC bit remains clear; HPET MSI not delivered; LAPIC ICR is single-CPU stub.
- FreeDOS/Linux helpers do **not** claim prompt/shell.
- ADR-0008 `etc/table-loader` still absent.
- No media → CF9 reboot loop rather than guest boot.
