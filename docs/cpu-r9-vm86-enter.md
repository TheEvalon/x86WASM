# Round 9 — VM86 enter via IRETD

## Shipped

* CPL-0 `IRETD` with `EFLAGS.VM=1` in the stack image enters virtual-8086 mode
  from the **9-dword** PL0 frame: EIP, CS, EFLAGS, ESP, SS, ES, DS, FS, GS
  (Intel SDM Vol. 2 "IRET/IRETD" RETURN-TO-VIRTUAL-8086-MODE; Vol. 3 §20.2
  Figure 20-4).
* Segment registers load with real-address bases (`selector << 4`).
* Architectural **CPL = 3** while `EFLAGS.VM=1` (Vol. 3 §5.5 / §20.1.1),
  including `PagedBus` and `require_cpl0` — CS[1:0] is **not** RPL in VM86.
* EIP above the 64 KiB real-mode CS limit → `#GP(0)` with no commit.
* Truncated frame → stack `#SS` before any VM86 state commit.

## Unsupported (explicit)

* VM86 interrupt/exception delivery that **builds** the 9-dword frame
* `IRET` while already in VM86 (slice 4)
* VME / PVI (`CR4.VME`/`CR4.PVI` remain reserved; CPUID does not advertise)
* Far JMP/CALL/INT and most protected-mode segment loads while `VM=1`
  (still branch on `CR0.PE` alone in many paths)
* Task-switch entry to VM86 (`EFLAGS.VM` in a new TSS)

## Spec

Intel SDM Vol. 2 "IRET/IRETD"; Vol. 3 §§5.5, 20.1–20.3; Vol. 1 §3.4.3.

## Files

* `crates/x86-interpreter/src/lib.rs` — `return_to_virtual_8086_mode`,
  `architectural_cpl`
* `crates/x86-interpreter/tests/cpu_r9_vm86_enter.rs`
* `docs/cpu-r9-vm86-enter.md`
