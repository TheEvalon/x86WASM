# Round 10 — Software INT n / INT3 / INTO from VM86 + IOPL

## Shipped

* Without VME: `INT n` and `INTO` (when OF=1) require `IOPL = 3`; otherwise
  `#GP(0)` (Vol. 3 §20.2.2 / Table 20-1).
* `INT3` is **not** IOPL-sensitive and still uses the VM86→CPL0 9-dword frame.
* Successful `INT n` / `INTO` deliver through a privilege-changing 386 IDT gate
  using the slice-1 VM86 frame path.
* `#GP(0)` from the IOPL check is itself delivered with the VM86 frame (+
  dword error code).

## Unsupported (explicit)

* VME interrupt redirection bitmap / soft-int redirect to IVT
* PVI
* Soft-int via 16-bit IDT gates from VM86
* ICEBP / INT1 (`F1`)

## Spec

Intel SDM Vol. 3 §20.2.2 Table 20-1; Vol. 2 INT n/INT3/INTO; Vol. 3 §§20.2–20.3.

## Files

* `crates/x86-interpreter/src/lib.rs` — IOPL gate on `0xCD` / `0xCE`
* `crates/x86-interpreter/tests/cpu_r10_vm86_soft_int_iopl.rs`
* `docs/cpu-r10-vm86-soft-int-iopl.md`
