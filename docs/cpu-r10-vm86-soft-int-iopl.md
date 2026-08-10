# Round 10 — Software INT n / INT3 / INTO from VM86 + IOPL

## Shipped

* Without VME: `INT n` requires `IOPL = 3`; otherwise `#GP(0)` (Vol. 3
  §20.2.2 / Table 20-2 method 2). The `#GP` is delivered with the VM86→CPL0
  frame plus a dword error code 0.
* `INT3` and `INTO` are **not** IOPL-sensitive (Vol. 2 INT n Virtual-8086 Mode
  Exceptions; 80386 PRM). They deliver through the IDT with the 9-dword frame
  even when `IOPL < 3`.
* Successful `INT n` / `INTO` / `INT3` use the slice-1 privilege-changing 386
  gate path.

## Unsupported (explicit)

* VME interrupt redirection bitmap / soft-int redirect to IVT
* PVI
* Soft-int via 16-bit IDT gates from VM86
* ICEBP / INT1 (`F1`)

## Spec

Intel SDM Vol. 3 §20.2.2 Table 20-2; Vol. 2 INT n/INT3/INTO; Vol. 3 §§20.2–20.3.

## Files

* `crates/x86-interpreter/src/lib.rs` — IOPL gate on `0xCD` only
* `crates/x86-interpreter/tests/cpu_r10_vm86_soft_int_iopl.rs`
* `docs/cpu-r10-vm86-soft-int-iopl.md`
