# Round 8 — Minimal VM86 (deferred) → ARPL substitute

## Decision

A bounded virtual-8086 slice was **deferred**. Prerequisites that are not yet
honestly wired:

* Forced CPL=3 while `EFLAGS.VM=1` (today CPL is taken from `CS.RPL`)
* IRETD 9-dword VM86 entry frame (ES/DS/FS/GS + SS:ESP + EFLAGS.VM)
* Privilege-changing IDT delivery that pushes the VM86 extended frame
  (GS/FS/DS/ES before SS:ESP) — current `deliver_protected_mode_gate` is
  protected-only Figure 6-5
* IOPL-sensitive instruction matrix (`CLI`/`STI`/`PUSHF`/`POPF`/`IN`/`OUT`/…)
* Decode / segment-load paths that still branch on `CR0.PE` alone

Shipping a partial enter that cannot exit or trap sensitive ops would violate
the project's "record unsupported / no invented behavior" rule.

## Substitute: ARPL (`63`)

Protected-mode `ARPL r/m16, r16` adjusts the destination RPL up to the source
RPL and sets ZF accordingly (Vol. 2 "ARPL"; Vol. 3 §5.4.3). Real-address mode
continues to raise `#UD`.

## Out of scope (still)

* Any `EFLAGS.VM=1` execution
* VME / PVI (`CR4` bits remain off; CPUID does not advertise them)

## Spec

Intel SDM Vol. 2 "ARPL"; Vol. 3 §§5.4.3, 20.1–20.3 (VM86 deferred).

## Files

* `crates/x86-spec/src/lib.rs` — primary `0x63` metadata
* `crates/x86-interpreter/src/lib.rs` — execute path
* `crates/x86-interpreter/tests/cpu_r8_arpl.rs`
* `docs/cpu-r8-vm86-minimal.md`
