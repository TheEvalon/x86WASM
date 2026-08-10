# Round 9 — PUSHF/POPF IOPL privilege (PM + VM86)

## Shipped

* Virtual-8086 mode without VME: `PUSHF`/`POPF` with `IOPL < 3` → `#GP(0)`
  (Intel SDM Vol. 3 §20.2.2; Vol. 2 "PUSHF"/"POPF").
* With `IOPL = 3`: execute; `POPF` may change `IF` but not `IOPL`/`VM`.
* Protected mode: `CPL > 0` cannot change `IOPL`; `IF` changes only when
  `CPL ≤ IOPL`. `VM`/`RF` never load from the image; RF cleared after `POPF`.

## Unsupported

* `CR4.VME` / VIP/VIF push-image masking
* `POPFQ` / `PUSHFQ`

## Spec

Intel SDM Vol. 2 "PUSHF/PUSHFD", "POPF/POPFD"; Vol. 3 §20.2.2.

## Files

* `crates/x86-interpreter/src/lib.rs` — `popf_execute`, VM86 `PUSHF` gate
* `crates/x86-interpreter/tests/cpu_r9_pushf_popf_iopl.rs`
* `docs/cpu-r9-pushf-popf-iopl.md`
