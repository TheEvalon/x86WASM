# Round 7 — LLDT / SLDT and LDT call-gate CALL

## Scope

* `LLDT` / `SLDT` (`0F 00 /2` / `/0`) load and store LDTR from a GDT LDT
  descriptor (`type=2`).
* Far `CALL` through a **32-bit call gate resident in the LDT** (`TI=1`),
  reusing the GDT call-gate path (param count 0).

## Behavior

* `LLDT` at CPL 0: null clears LDTR; present type-2 caches base/limit/AR;
  wrong type / `TI=1` source / not-present → `#GP`/`#NP`; real mode → `#UD`.
* `SLDT` stores the visible LDTR selector at any CPL (and in real mode).
* LDT call-gate `CALL` reads the gate from `LDTR.base`; target code remains
  GDT-only in this slice.

## Out of scope

* LDT-resident target code segments for the gate
* LDT resolution for `VERR`/`VERW`/`LAR`/`LSL`/ordinary Sreg loads
* Non-zero call-gate parameter count

## Spec

Intel SDM Vol. 2 "LLDT"/"SLDT"/"CALL"; Vol. 3 §§2.4.2, 3.5.1–3.5.2, 5.8.2.

## Files

* `crates/x86-interpreter/src/lib.rs`
* `crates/x86-spec/src/lib.rs` — Group 6 comment
* `crates/x86-interpreter/tests/cpu_r7_lldt_ldt_call_gate.rs`
* `docs/cpu-r7-lldt-ldt-call-gate.md`
