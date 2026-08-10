# Round 13 — VME `CLI`/`STI` → `VIF`

## Shipped

* With `CR4.VME=1`, `EFLAGS.VM=1`, and `IOPL < 3`:
  * `CLI` clears `VIF` (bit 19); leaves `IF` unchanged.
  * `STI` sets `VIF`; leaves `IF` unchanged.
  * `STI` while `VIP=1` → `#GP(0)` (VIP∧VIF).
* With `IOPL = 3` under VME: `CLI`/`STI` still operate on `IF`; `STI` while
  `VIP=1` → `#GP(0)` (VIP∧IF).
* Without `CR4.VME`: R9 contract unchanged (`IOPL < 3` → `#GP`).
* `CPUID.01H:EDX.VME` remains **clear**.

## Unsupported (explicit)

* `CPUID.VME` / `CR4.PVI` / PVI `CLI`/`STI` on VIF
* STI interrupt-shadow (one-instruction delay before maskable IRQs see IF/VIF)
* Hardware IRQ sampling of VIF (pending VIP injection path)
* `PUSHF`/`POPF` VIF image rewrite (Round-13 slice 2)

## Spec

Intel SDM Vol. 2 "CLI"/"STI"; Vol. 3 §§20.2–20.3 Table 20-2; Vol. 3 §2.5.

## Files

* `crates/x86-interpreter/src/lib.rs` — `cli_sti_execute`, `EFLAGS_VIF`/`VIP`
* `crates/x86-interpreter/tests/cpu_r13_vme_cli_sti_vif.rs`
* `docs/cpu-r13-vme-cli-sti-vif.md`
