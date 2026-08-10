# Round 12 — CR4.VME enable honesty without CPUID.VME

## Shipped

* `CR4.VME` (bit 0) is **guest-writable** and sticky through `MOV to/from CR4`.
* `CPUID.01H:EDX[1]` (`VME`) remains **clear**. Full Virtual-8086 Mode
  Extensions (VIF/VIP, `CLI`/`STI` on VIF, complete Table 20-2) are not claimed.
* `CR4.PVI` and other unimplemented CR4 bits stay reserved (`#GP(0)` on write of 1).

## Honesty note

SDM Vol. 3 §4.1.4 normally couples `CR4.VME` to `CPUID.VME`. This emulator
allows the CR4 bit so later Round-12 slices can exercise the interrupt-
redirection bitmap stub, while refusing to advertise the unfinished feature
in CPUID (`AGENTS.md`).

## Unsupported (explicit)

* `CPUID.VME` / `CPUID.PVI`
* VIF-based `CLI`/`STI`, VIP∧VIF `#GP`
* Full VME soft-int methods beyond the Round-12 redirect stub

## Spec

Intel SDM Vol. 3 §2.5 (CR4); Vol. 2 MOV CRn / CPUID; Vol. 3 §4.1.4.

## Files

* `crates/x86-interpreter/src/lib.rs` — `CR4_VME` in `cr4_reserved_mask`
* `crates/x86-interpreter/tests/cpu_r12_cr4_vme_honesty.rs`
* `crates/x86-interpreter/tests/cpu_r4_control_registers.rs` — reserved probe list
* `docs/cpu-r12-cr4-vme-honesty.md`
