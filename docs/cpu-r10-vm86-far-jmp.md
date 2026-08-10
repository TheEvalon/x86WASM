# Round 10 — Far JMP while VM=1

## Shipped

* Direct far `JMP ptr16:16` (`EA`) in virtual-8086 mode reloads CS:IP with
  real-address bases (`selector << 4`) and **stays** in VM86 (`EFLAGS.VM`
  unchanged).
* Indirect far `JMP m16:16` (Group 5 `/5`) same behavior.

## Unsupported (explicit)

* Operand-size 32 far JMP (`ptr16:32` / `m16:32`) while VM=1 — truncated like
  real-address mode if reached via `66H`; not a privileged exit path
* Call gates / task gates as far JMP targets from VM86 (use protected path only
  when `VM=0`)
* VME

## Spec

Intel SDM Vol. 3 §20.1 / §20.1.3; Vol. 2 "JMP"; Vol. 3 §3.4.2.

## Files

* `crates/x86-interpreter/src/lib.rs` — `0xEA` / Group 5 `/5` VM86 branch
* `crates/x86-interpreter/tests/cpu_r10_vm86_far_jmp.rs`
* `docs/cpu-r10-vm86-far-jmp.md`
