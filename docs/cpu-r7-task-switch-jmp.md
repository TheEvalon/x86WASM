# Round 7 — JMP-form hardware task switch (32-bit TSS / task gate)

## Scope

Protected-mode far `JMP` (`EA` / `FF /5`) to:

* a present **available 32-bit TSS** (`type=9`) in the GDT, or
* a present **task gate** (`type=5`) whose selector names such a TSS

performs a JMP-form hardware task switch (Vol. 3 §§7.2–7.3).

## Behavior

* Outgoing state (EIP=next, EFLAGS, GPRs, segment selectors, LDTR, CR3) is
  written into the current busy TSS; that descriptor becomes available.
* Incoming state is loaded from the new TSS; the descriptor becomes busy;
  `CR0.TS` is set; `NT` is cleared in the loaded EFLAGS image.
* Privilege checks follow Figure 7-5 (direct TSS vs task gate).
* Busy / wrong-type / not-present / short-limit targets raise `#GP`/`#NP`.

## Out of scope

* Nested-task `CALL` / `INT` through TSS or task gate (`NT=1`, back-link)
* IDT task-gate exception delivery
* `EFLAGS.VM=1` (VM86) targets — reported as `Unsupported`
* 16-bit TSS (`type=1`)
* LDT-resident TSS or task-gate descriptors

## Spec

Intel SDM Vol. 2 "JMP"; Vol. 3 §§7.2–7.3 (Table 3-2, Figure 7-2, Figure 7-5).

## Files

* `crates/x86-interpreter/src/lib.rs` — switch helpers + far JMP/CALL hooks
* `crates/x86-interpreter/tests/cpu_r7_task_switch_jmp.rs`
* `docs/cpu-r7-task-switch-jmp.md`
