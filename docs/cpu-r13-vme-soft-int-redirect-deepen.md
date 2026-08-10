# Round 13 — soft-int redirect bitmap deepen

## Shipped

* Method 6 (`CR4.VME=1`, `VM=1`, redirect bit clear, `IOPL < 3`):
  pushed FLAGS image has `IOPL := 3` and `IF ← VIF`; processor clears
  `IF`/`TF`/`VIF` after redirect.
* Method 5 (`IOPL = 3`): push live FLAGS; clear `IF`/`TF` only (VIF sticky).
* Edge coverage: vectors `0x00` / `0x07` / `0xFF`; incomplete map
  (`TR.limit` too small); `I/O map base < 32` (no silent IVT invent).
* R12 `INT 21h` IOPL=0 case updated to expect method-6 rewrite.

## Unsupported (explicit)

* Hardware IRQ / exception redirection via the bitmap
* `INT3` / `INTO` redirection (bitmap applies only to `INT n`)
* `CPUID.VME`
* VIP pending-injection when VIF is set by the guest

## Spec

Intel SDM Vol. 3 §§20.2–20.3 Table 20-2 / Figure 20-5; Vol. 3 §7.2.1; Vol. 2 INT n.

## Files

* `crates/x86-interpreter/src/lib.rs` — `vm86_vme_redirect_soft_int` method 5/6
* `crates/x86-interpreter/tests/cpu_r13_vme_soft_int_redirect_deepen.rs`
* `crates/x86-interpreter/tests/cpu_r12_vme_soft_int_redirect.rs` — method-6 expect
* `docs/cpu-r13-vme-soft-int-redirect-deepen.md`
