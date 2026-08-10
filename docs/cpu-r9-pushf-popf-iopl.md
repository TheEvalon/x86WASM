# Round 9 — PUSHF/POPF IOPL privilege

## Shipped

* VM86 without VME: `PUSHF`/`POPF` with `IOPL < 3` → `#GP(0)` (Vol. 3 §20.2.2).
* `PUSHF` reflects the live `IOPL` field on the stack (PM and VM86 when allowed).
* Protected-mode `POPF`: CPL>0 cannot change `IOPL`; `IF` changes only when
  `CPL ≤ IOPL`. No exception — privileged bits are ignored (Vol. 2 POPF).
* `VM` never loads from a `POPF` image; `RF` is cleared after `POPF`.

## Unsupported

* VME 16-bit `POPF` with `IOPL < 3` (VIF path)
* `POPFQ` / 64-bit mode
* VIP/VIF modification from `POPF` at CPL 0 (left sticky/unmodified)

## Spec

Intel SDM Vol. 2 "PUSHF/PUSHFD", "POPF/POPFD"; Vol. 3 §20.2.2; Vol. 1 §3.4.3.

## Files

* `crates/x86-interpreter/src/lib.rs` — `popf_execute`
* `crates/x86-interpreter/tests/cpu_r9_pushf_popf_iopl.rs`
* `docs/cpu-r9-pushf-popf-iopl.md`
