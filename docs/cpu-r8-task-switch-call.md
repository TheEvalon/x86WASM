# Round 8 — CALL-form hardware task switch (32-bit TSS / task gate)

## Scope

Protected-mode far `CALL` (`9A` / `FF /3`) to:

* a present **available 32-bit TSS** (`type=9`) in the GDT, or
* a present **task gate** (`type=5`) whose selector names such a TSS

performs a CALL-form (nested) hardware task switch (Vol. 3 §§7.2–7.3 Table 7-1).

## Behavior

* Outgoing state (EIP=next, EFLAGS, GPRs, segment selectors, LDTR, CR3) is
  written into the current busy TSS; that descriptor **stays busy**.
* The new TSS previous-task link (offset 0) is set to the old TR selector.
* Incoming state is loaded from the new TSS; the descriptor becomes busy;
  `CR0.TS` is set; `NT` is **set** in the loaded EFLAGS image.
* Privilege checks follow Figure 7-5 (same as JMP for direct TSS / task gate).
* Busy / wrong-type / not-present / short-limit targets raise `#GP`/`#NP`.

## Out of scope

* Nested-task `INT` / exception through TSS or IDT task gate
* `IRET` with `NT=1` (Round-8 slice 2)
* `EFLAGS.VM=1` (VM86) targets — reported as `Unsupported`
* 16-bit TSS (`type=1`)
* LDT-resident TSS or task-gate descriptors

## Spec

Intel SDM Vol. 2 "CALL"; Vol. 3 §§7.2–7.3 (Table 3-2, Figure 7-2, Figure 7-5,
Table 7-1).

## Files

* `crates/x86-interpreter/src/lib.rs` — shared `task_switch` + CALL hook
* `crates/x86-interpreter/tests/cpu_r8_task_switch_call.rs`
* `docs/cpu-r8-task-switch-call.md`
