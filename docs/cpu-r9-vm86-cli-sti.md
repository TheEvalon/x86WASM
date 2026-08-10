# Round 9 — VM86-sensitive CLI/STI

## Shipped

* Virtual-8086 mode without VME: `CLI`/`STI` with `IOPL < 3` → `#GP(0)`;
  with `IOPL = 3` update `IF` (Intel SDM Vol. 2 "CLI"/"STI" Table 3-7/3-8;
  Vol. 3 §20.2.1 / ch.20).
* Protected mode without PVI: `CPL > IOPL` → `#GP(0)` for `CLI`/`STI`.
* Privilege uses `architectural_cpl` (VM forces CPL 3).

## Unsupported

* `CR4.VME` / `CR4.PVI` and the VIF alternate path (CPUID does not advertise)
* STI interrupt shadow / one-instruction delay before IF takes effect for IRQs

## Spec

Intel SDM Vol. 2 "CLI", "STI"; Vol. 3 §§20.1–20.2.

## Files

* `crates/x86-interpreter/src/lib.rs` — `require_iopl_for_cli_sti`
* `crates/x86-interpreter/tests/cpu_r9_vm86_cli_sti.rs`
* `docs/cpu-r9-vm86-cli-sti.md`
