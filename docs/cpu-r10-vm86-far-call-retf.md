# Round 10 — Far CALL / RETF while VM=1

## Shipped

* Direct far `CALL ptr16:16` (`9A`) in VM86 pushes CS then IP (16-bit),
  reloads CS:IP real-mode-like, **stays** `EFLAGS.VM=1`.
* Indirect far `CALL m16:16` (Group 5 `/3`) same.
* `RETF` (`CB`) pops IP/CS with real-mode CS load; stays in VM86 (already
  real-mode-like for this path).

## Unsupported (explicit)

* Privilege-changing far CALL from VM86 via call gate (document; do not
  silently take protected call-gate path while `VM=1`)
* Operand-size 32 far CALL/RETF frames while VM=1 (truncated / opaque)
* Task-switch CALL from VM86

## Spec

Intel SDM Vol. 3 §20.1; Vol. 2 "CALL" / "RET" (far); Vol. 3 §3.4.2.

## Files

* `crates/x86-interpreter/src/lib.rs` — `0x9A` / Group 5 `/3` VM86 branch
* `crates/x86-interpreter/tests/cpu_r10_vm86_far_call_retf.rs`
* `docs/cpu-r10-vm86-far-call-retf.md`
