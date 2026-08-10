# Round 8 — IRET/IRETD nested-task return (`NT=1`)

## Scope

Protected-mode `IRET` / `IRETD` (`CF`) with `EFLAGS.NT=1` performs an
IRET-form hardware task switch to the TSS named by the current TSS
previous-task link (Vol. 3 §7.3 / Vol. 2 IRET nested-task operation).

## Behavior

* No stack frame is popped.
* Outgoing state is saved into the current busy TSS; that descriptor becomes
  **available**.
* Incoming state is loaded from the linked TSS, which must already be **busy**.
* `NT` is cleared in the loaded EFLAGS image.
* `CR0.TS` is set.

## Out of scope

* Virtual-8086 `IRET` (`EFLAGS.VM=1` current or return image)
* Nested-task entry via `INT` / IDT task gate
* 16-bit TSS back-links
* Returning to a busy TSS that fails segment validation mid-load
  (same bounded segment rules as JMP/CALL switches)

## Spec

Intel SDM Vol. 2 "IRET/IRETD"; Vol. 3 §§7.2–7.3 Table 7-1.

## Files

* `crates/x86-interpreter/src/lib.rs` — `protected_iret` NT path → `task_switch`
* `crates/x86-interpreter/tests/cpu_r8_iret_nt.rs`
* `docs/cpu-r8-iret-nt.md`
