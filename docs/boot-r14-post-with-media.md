# R14 POST measure with INT 19h bootable HD

Milestone 2, round 14, boot-guest lane, slice 1.

## Method

```text
Machine::with_bios_rom(32MiB, seabios)
  .attach_bootable_hd_for_int19()   # R13 synthetic INT19 HD
  .measure_post_with_bootable_hd(20_000_000)
```

Budget matches the no-media CF9 remeasure (`docs/post-c897-remeasure.md`).
Short-budget harness validates attach/classify; full remeasure via
`X86WASM_POST_MEDIA_FULL=1` (~211s debug).

## Scope

| Piece | Role |
|---|---|
| `POST_WITH_MEDIA_BUDGET_STEPS` | `20_000_000` |
| `NO_MEDIA_REBOOT_CLASS_CS/IP` | `F000:9842` |
| `classify_post_with_media_stop` | still-no-media / guest-halted-at-boot-sector / other-stop |
| `Machine::measure_post_with_bootable_hd` | Attach INT19 HD + `probe_post` |

## Results (2026-08-11, this branch)

```text
post-with-media: budget=20000000 media=hd-active-partition:lba=1 type=0x01
                 int19-candidate=true class=other-stop:F000:C897
post-probe: steps=11810688 stop=step-budget-exhausted
  stop-pc        cs:ip=F000:C897 …
  halt-idle      idle-pct≈40%
```

| Metric | No-media (R12 CF9, 20M) | With INT19 HD (R14, 20M) |
|---|---|---|
| Media | none | `hd-active-partition` LBA1 FAT12 |
| Stop CS:IP | `F000:9842` | `F000:C897` |
| Class | still-no-media-reboot-class | **other-stop** (past `9842` class) |
| INT19 / `0000:7C00` halt? | n/a (reboot loop) | **not observed** |

Interpretation: attaching INT19-candidate media leaves the documented
**no-media** `F000:9842` reboot-loop class within the same 20M budget. The stop
lands in the late-POST `wait_irq` site (`F000:C897`) instead — firmware path
differs when a disk is present, but this run does **not** show a guest boot
sector halt at `0000:7C00`. Larger budgets / guest INT 13h remain follow-ons.

## Honesty

- **Not** SeaBIOS INT 19h success (no `0000:7C00` halt measured).
- Synthetic HLT MBR is a media candidacy fixture, not FreeDOS/Linux.
- Host `attach_bootable_*` is not a guest disk BIOS.

## Spec

IBM PC BIOS INT 19h / OSDev Boot Sequence; ICH CF9 path
(`docs/post-c897-remeasure.md`); R13 media helpers
(`docs/boot-r13-int19-bootable-media.md`).
