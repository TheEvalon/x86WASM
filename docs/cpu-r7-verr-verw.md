# Round 7 — VERR / VERW

## Scope

Protected-mode `VERR` (`0F 00 /4`) and `VERW` (`0F 00 /5`): soft segment
checks that set `ZF` without loading a segment register.

## Behavior

* Real-address mode → `#UD`.
* Null selector, LDT (`TI=1`), out-of-GDT-limit, not-present, or failed type /
  privilege check → `ZF=0`.
* `VERR`: present readable data or readable code; conforming code skips DPL.
* `VERW`: present writable data only (`W=1`); code and system types clear ZF.
* Privilege: `CPL ≤ DPL` and `RPL ≤ DPL` (except conforming `VERR`).

## Out of scope

* LDT resolution (`TI=1` clears ZF)
* `LOCK` `#UD`

## Spec

Intel SDM Vol. 2 "VERR"/"VERW".

## Files

* `crates/x86-spec` — Group 6 comment
* `crates/x86-interpreter` — `exec_verr_verw` + Group 6 `/4`/`/5`
* `crates/x86-interpreter/tests/cpu_r7_verr_verw.rs`
* `docs/cpu-r7-verr-verw.md`
