# Round 12 — 16-bit IDT gate from VM86

## Shipped

* Privilege-changing **286** interrupt/trap gates (`type 6/7`) from VM86 push
  the **9-word** frame: GS, FS, DS, ES, SS, SP, FLAGS, CS, IP (low 16 bits of
  each architectural value). Destination `SS.B` still selects SP vs ESP width
  for the pointer updates.
* DS/ES/FS/GS are nullified after delivery; `EFLAGS.VM`/`TF`/`NT`/`RF` cleared;
  interrupt gates clear `IF`.
* Non-VM86 rejection of 16-bit gates from a `CS.D=1` current code segment is
  unchanged (would truncate EIP).

## Unsupported (explicit)

* Same-CPL delivery from VM86 (handler DPL=3)
* Task gates from VM86
* 16-bit outer `IRET` back into VM86 (needs `IRETD` / 32-bit image for `VM`)

## Spec

Intel SDM Vol. 3 §§20.2–20.3, §6.11–§6.12.1; Vol. 2 INT n.

## Files

* `crates/x86-interpreter/src/lib.rs` — `deliver_protected_mode_gate`
* `crates/x86-interpreter/tests/cpu_r12_vm86_16bit_idt_gate.rs`
* `docs/cpu-r12-vm86-16bit-idt-gate.md`
