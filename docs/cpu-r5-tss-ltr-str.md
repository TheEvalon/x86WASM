# TSS descriptor load: `LTR` / `STR` (Milestone 2, round 5)

Bounded slice: load a **32-bit available TSS** into `TR` and store the
selector with `STR`. No hardware task switch.

## Authority

| Rule | Section |
|---|---|
| 32-bit TSS format; minimum limit `67H` | Vol. 3 §7.2.1 |
| Available vs busy TSS types (`9` / `B`) | Vol. 3 Table 3-2, §7.2.2 |
| `LTR` loads TR and marks the TSS busy | Vol. 3 §7.3; Vol. 2 "LTR" |
| `STR` stores the visible TR selector | Vol. 2 "STR" |
| System-table accesses are supervisor | Vol. 3 §4.6.1 |

No implementation from another emulator was read or copied.

## Supported

* `0F 00 /3 LTR r/m16` in protected mode at CPL 0: null/`TI=1`/wrong-type/
  busy/`limit < 67H` → `#GP(selector)` (null → `#GP(0)`); not present →
  `#NP(selector)`; success caches base/limit/AR in `TR` and writes type `B`
  into the GDT descriptor.
* `0F 00 /1 STR r/m16` stores `TR.selector` at any CPL (and in real mode).
* `Bus::write_system_u8` for the busy-bit update (supervisor access).

## Not supported

* 16-bit TSS (`type=1`), busy-TSS load via task switch, `LLDT`/`SLDT`,
  `VERR`/`VERW`, hardware task gates / task switches, VM86.
* Using `SS0:ESP0` from the TSS (next slice).
