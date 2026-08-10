# Round 10 — VM86 → CPL0 interrupt/exception frame

## Shipped

* Privilege-changing 386 interrupt/trap gate delivery while `EFLAGS.VM=1`
  uses architectural CPL 3 (not `CS[1:0]`) and switches to TSS `SS0:ESP0`.
* Pushes the **9-dword** PL0 frame (low→high): EIP, CS, EFLAGS(with VM),
  ESP, SS, ES, DS, FS, GS (Intel SDM Vol. 3 §20.2 Figure 20-2 / §6.12.1).
* Nullifies DS/ES/FS/GS after the stack switch (Vol. 3 §20.2).
* Clears VM/TF/NT/RF; interrupt gates clear IF; trap gates preserve IF.
* Covered via `INT3` (not IOPL-sensitive), trap-gate IF preserve, and `#UD`.

## Unsupported (explicit)

* VME / PVI (`CR4.VME`/`CR4.PVI`; CPUID does not advertise)
* 16-bit (286) IDT gates from VM86
* Same-CPL VM86 delivery (handler code DPL=3)
* Task gates / nested-task VM86 entry
* Full TSS SS0/ESP0 edge cases beyond existing TSS support

## Spec

Intel SDM Vol. 3 §§20.2–20.3, 6.12.1; Vol. 2 INT n/INT3/INTO; Vol. 1 §3.4.3.

## Files

* `crates/x86-interpreter/src/lib.rs` — `deliver_protected_mode_gate`
* `crates/x86-interpreter/tests/cpu_r10_vm86_int_frame.rs`
* `docs/cpu-r10-vm86-int-frame.md`
