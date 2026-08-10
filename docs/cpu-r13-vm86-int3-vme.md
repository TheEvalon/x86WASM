# Round 13 — `INT3` under VME (no redirect) + TF skip

## Shipped

* With `CR4.VME=1` and `VM=1`, `INT3` (`0xCC`) **ignores** the software-
  interrupt redirection bitmap (even when vector-3 bit is clear) and delivers
  through the protected-mode IDT with the VM86→CPL0 9-dword frame.
* Contrast: `INT 3` (`CD 03`) **does** redirect when the bit is clear.
* `INT3` remains non-IOPL-sensitive at `IOPL=0` under VME.
* Comments updated; tests lock the `CC` vs `CD 03` distinction.

## Explicitly skipped / unsupported

* **Single-step trap (`TF` / `#DB`) under VME** — not implemented this round
  (no VME-aware `#DB` method table; no VIF interaction). Deferred.
* `ICEBP`/`INT1` (`F1`) — still a decode miss (not silent `#DB`).
* `CPUID.VME` remains clear.
* Hardware IRQ redirection / VIP pending injection.

## Spec

Intel SDM Vol. 2 "INT n/INTO/INT3/INT1"; Vol. 3 §20.2.2 Table 20-2;
Vol. 3 Table 20-1 (INT3 not IOPL-sensitive; bitmap is `INT n` only).

## Files

* `crates/x86-interpreter/src/lib.rs` — `INT3` comment honesty
* `crates/x86-interpreter/tests/cpu_r13_vm86_int3_vme.rs`
* `docs/cpu-r13-vm86-int3-vme.md`
