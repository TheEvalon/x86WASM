# Round 11 — VM86 PUSH/POP segment registers

## Shipped

* While `EFLAGS.VM=1`, primary-map `PUSH`/`POP` of `ES`/`CS`/`SS`/`DS`
  (`06`/`0E`/`16`/`1E`, `07`/`17`/`1F`) and two-byte `PUSH`/`POP` `FS`/`GS`
  (`0F A0`/`A1`/`A8`/`A9`) are **real-address-like**: PUSH writes the
  selector; POP sets `base = selector << 4` and does **not** consult the GDT.
* Execution remains in virtual-8086 mode (`VM` sticky).
* `MOV` Sreg and `LDS`/`LES`/`LSS`/`LFS`/`LGS` share the same VM86 load rule
  via the shared helpers (still not a privilege exit).

## Unsupported (explicit)

* VME / PVI
* Protected-mode descriptor checks while `VM=1` (intentionally absent)
* Expanding `SS.B=1` stack address size from a VM86 POP of a protected-style
  descriptor (VM86 uses real-mode cache shape)

## Spec

Intel SDM Vol. 3 §20.1 / §20.1.1; Vol. 2 "PUSH" / "POP"; Vol. 3 §3.4.2.

## Files

* `crates/x86-interpreter/src/lib.rs` — `prepare_sreg_load` / `write_sreg` /
  `LDS`/`LES` VM86 branch
* `crates/x86-interpreter/tests/cpu_r11_vm86_push_pop_sreg.rs`
* `docs/cpu-r11-vm86-push-pop-sreg.md`
