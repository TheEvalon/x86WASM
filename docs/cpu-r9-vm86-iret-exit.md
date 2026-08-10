# Round 9 — VM86 IRET exit / #GP

## Shipped

* `IRET`/`IRETD` while `EFLAGS.VM=1` and `IOPL < 3` → `#GP(0)` (Vol. 2
  RETURN-FROM-VIRTUAL-8086-MODE; Vol. 3 §20.2.3).
* With `IOPL = 3`: real-mode-like pop; **stays in VM86** (`VM`/`IOPL`/`VIP`/`VIF`
  sticky).
* **Successful exit case:** after enter, a CPL-0 monitor context executes
  `IRETD` with `VM=0` in the EFLAGS image and returns to protected mode
  (same-CPL ring-0 frame). This models the monitor half of exit without
  implementing VM86→CPL0 interrupt delivery.

## Unsupported (honesty)

* Privilege-changing IDT delivery from VM86 that **builds** the 9-dword PL0
  frame (GS/FS/DS/ES + SS:ESP + EFLAGS.VM) — still out of scope
* Nested-task return involving VM86
* VME redirect of IOPL-sensitive IRET
* Task-switch leave/enter VM86

## Spec

Intel SDM Vol. 2 "IRET/IRETD"; Vol. 3 §§20.2–20.3.

## Files

* `crates/x86-interpreter/src/lib.rs` — `vm86_iret`
* `crates/x86-interpreter/tests/cpu_r9_vm86_iret_exit.rs`
* `docs/cpu-r9-vm86-iret-exit.md`
