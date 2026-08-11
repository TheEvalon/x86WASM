# POST-with-media remeasure — Milestone 2 Round 15 (platform-post)

## Ownership

Platform-post owns IRQ/timer device honesty for SeaBIOS `wait_irq` (PIT IRQ0,
PIC IRR/ISR/OCW, CMOS IRQ8). **Boot-guest** owns full C897-with-media diagnosis
and INT19/`7C00` handoff classification (`docs/boot-r14-post-with-media.md`).

## Baseline (R14, unchanged class)

```text
post-with-media: budget=20000000 media=hd-active-partition …
                 class=other-stop:F000:C897
halt-idle ≈40%
```

`F000:C897` is SeaBIOS `wait_irq` (`sti; hlt; cli; …`) sampled mid-yield — not
an opcode gap (`docs/post-c897-cf9-diagnosis.md`).

## R15 platform slices landed

| Slice | Doc | Device effect on wait_irq |
|---|---|---|
| PIT IRQ0 deepen | `docs/platform-r15-pit-irq0-wait-irq.md` | Mode-2 OUT edges re-latch edge IR0 after EOI; two SeaBIOS-shaped yields wake |
| PIC IRR/ISR OCW | `docs/platform-r15-pic-irr-isr-ocw.md` | Sticky OCW3 ISR/IRR around IRQ0 EOI; cascade IRQ8 OCW3 view |
| RTC IRQ8 enable | `docs/platform-r15-rtc-irq8.md` | PIE-on-latched-PF + Status C clear sync PIC IR8 immediately |

## Remeasure verdict (this lane)

**C897-with-media did not improve** as a stop class: these fixes deepen the
IRQ0/IRQ8 path that already makes `wait_irq` yields idle (~40% halt-idle in
R14). They do **not** move the 20M POST-with-media stop off `F000:C897`.

Causal gap remaining (platform view):

- Stop PC is still a **budget sampling artifact** inside healthy yields, not a
  missing IRQ0 edge or OCW3 bug.
- Past C897 with media requires boot-path progress (INT19 / disk BIOS /
  guest handoff) owned by the boot lane — not further classic PIC/PIT polish.

Device-level evidence (this branch): `wait_irq_pit`, mode-2 re-latch after EOI,
CMOS PIE→IRQ8 port sync tests are green. Full 20M SeaBIOS remeasure remains the
boot lane's formal POST-with-media harness when coordinating merge.

## Unsupported / honesty

- Do not claim POST complete or INT19/`0000:7C00` success from this lane
- Host-real-time PIT/RTC rates remain model choices (step clock)
- No guest INT 08h/0Ah bodies

## Spec / priors

- SeaBIOS rel-1.16.3 `wait_irq` / `check_irqs` / `clock_poll_irq`
- `docs/boot-r14-post-with-media.md`, `docs/post-c897-*.md`
