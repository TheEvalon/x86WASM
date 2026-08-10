# Round 13 — `PUSHF`/`POPF` VIF/VIP under VME

## Shipped

* With `CR4.VME=1`, `VM=1`, `IOPL < 3` (Table 20-2 method 4):
  * `PUSHF`/`PUSHFD` do **not** `#GP`; pushed image reports `IOPL=3` and
    `IF ← VIF` (live `IF`/`IOPL` unchanged).
  * `POPF`/`POPFD` do **not** `#GP`; image `IF` updates `VIF` (not
    architectural `IF`); `IOPL` sticky; high-word `VIP`/`VIF` never load from
    the image (consistent with R11 sticky bits).
  * Enabling `VIF` (image `IF=1`) while `VIP=1` → `#GP(0)` before SP commit.
* With `IOPL = 3` under VME: ordinary `IF` load; VIP∧IF on enable → `#GP(0)`.
* Without VME: R9 `#GP` on `IOPL < 3` unchanged.
* `CPUID.01H:EDX.VME` remains clear.

## Unsupported (explicit)

* `CPUID.VME` / `CR4.PVI` / PVI flag masking
* `PUSHFQ`/`POPFQ`
* Method-6 soft-int FLAGS rewrite (slice 3)
* Hardware VIP injection / VIF-sampled IRQs

## Spec

Intel SDM Vol. 2 "PUSHF/PUSHFD", "POPF/POPFD"; Vol. 3 §§20.2–20.3 Table 20-2.

## Files

* `crates/x86-interpreter/src/lib.rs` — `pushf_image`, `popf_execute` VME path
* `crates/x86-interpreter/tests/cpu_r13_vme_pushf_popf_vif.rs`
* `docs/cpu-r13-vme-pushf-popf-vif.md`
