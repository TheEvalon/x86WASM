# R15 POST-with-media stop at `F000:C897`

Milestone 2, round 15, boot-guest lane, slice 1.

## Symptom (R14 measured)

| Path | Budget | Stop CS:IP | Class (R14) |
|---|---|---|---|
| No media | 20M | `F000:9842` | no-media reboot / `boot_fail` |
| INT19-candidate HD | 20M | `F000:C897` | `other-stop` (past 9842) |

No `0000:7C00` guest boot-sector halt observed with media under 20M
(`docs/boot-r14-post-with-media.md`).

## Evidence

1. **ROM at `F000:C895`:** `FB F4 FA FC 66 C3` = `sti; hlt; cli; cld; ret` —
   SeaBIOS `wait_irq` (`src/stacks.c` rel-1.16.3). Stop IP `C897` is the `cli`
   after HLT when the step budget ends mid-yield — **not** a decode/#UD gap
   (`docs/post-c897-cf9-diagnosis.md`).

2. **No-media contrast:** With CF9 live and **no** boot media, the same 20M
   budget reaches `boot_fail` → CF9 reboot and samples `F000:9842`
   (`docs/post-c897-remeasure.md`). Media changes the firmware path enough that
   the budget expires **earlier** in late POST (still yielding in `wait_irq`)
   rather than in the no-media reboot loop.

3. **Idle share:** R14 media remeasure reported ~40% halt-idle — consistent with
   `wait_irq` HLT yields, not a tight busy spin at a missing opcode.

## R15 classify (this slice)

| Piece | Role |
|---|---|
| `WAIT_IRQ_CLASS_CS/IP` | `F000:C897` |
| `PostMediaActivity` | `idle-dominant` / `busy-dominant` (threshold 35%) |
| `PostMediaRebootSignal` | `wait-irq-no-reboot-yet` vs `reboot-pulse-seen:N` (CF9) |
| `PostWithMediaClass::WaitIrqYield` | replaces bare `other-stop` at C897 |
| `Machine::measure_post_with_bootable_hd` | records `cf9_pulses` + refined class |

## Causal interpretation (ownership)

| Layer | Finding |
|---|---|
| `F000:C897` with media | Sampling of `wait_irq` during **disk-present** late POST yields |
| Vs `F000:9842` | Media path has **not** completed INT 19h → `boot_fail` within 20M |
| Guest `0000:7C00` | **Not** observed — SeaBIOS INT 19h handoff not measured |
| Boot-guest ownership | Classify + measure + docs; no CF9/PIC/IDE rewrite here |
| Follow-on | **Platform:** IRQ/timer progress during disk-probe yields; **Storage:** guest INT 13h AH=02/42 depth for SeaBIOS disk scan |

## Honesty / unsupported

- **Not** SeaBIOS POST complete or INT 19h success.
- **Not** an opcode gap at `C897`.
- Zero CF9 pulses at C897 means mid-yield sampling, not “POST failed”.
- Full 20M remeasure remains opt-in (`X86WASM_POST_MEDIA_FULL=1`).

## Spec / sources

SeaBIOS rel-1.16.3 `wait_irq` / `boot_fail`; IBM PC BIOS INT 19h; ICH CF9
(`docs/post-c897-*.md`); R14 media measure (`docs/boot-r14-post-with-media.md`).
