# Round 11 — VIP/VIF honesty without VME

## Shipped

* Without `CR4.VME` (and with CPUID.1:EDX.VME clear): `POPF`/`POPFD` and
  `IRET`/`IRETD` in virtual-8086 mode **never** load `VIP` (bit 20) or `VIF`
  (bit 19) from the stack image; those bits are sticky along with `VM`/`IOPL`
  (Vol. 2 POPF / RETURN-FROM-VIRTUAL-8086-MODE; Vol. 3 Table 20-2 method 2).
* No VME redirect is invented: `CLI`/`STI` still operate on `IF` when
  `IOPL = 3`; they do **not** toggle `VIF`. Enabling `IF` while `VIP=1` does
  **not** raise `#GP(0)` (that `#GP` is a VME feature).

## Unsupported (explicit — do not invent)

* `CR4.VME` / `CR4.PVI` (CR4 reserved bits stay clear; writes of 1 `#GP`)
* VME interrupt-redirection bitmap / soft-int → IVT
* VIF-based `CLI`/`STI`, VIP∧VIF `#GP` on interrupt enable
* CPUID feature bit `VME` (EDX bit 1) — remains clear

## Spec

Intel SDM Vol. 2 "POPF/POPFD", "IRET/IRETD"; Vol. 3 §20.2 / Table 20-2;
Vol. 3 §2.5 (CR4.VME); Vol. 2 CPUID.

## Files

* `crates/x86-interpreter/src/lib.rs` — `popf_execute`, `vm86_iret` (comments)
* `crates/x86-interpreter/tests/cpu_r11_vm86_vip_vif_no_vme.rs`
* `docs/cpu-r11-vm86-vip-vif-no-vme.md`
