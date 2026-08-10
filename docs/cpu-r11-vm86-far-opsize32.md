# Round 11 — Opsize-32 far JMP / CALL / RETF while VM=1

## Shipped

* With operand size 32 (`66H` under VM86 `CS.D=0`), far transfers use the
  real-address-like subset:
  * Direct `JMP ptr16:32` (`66 EA`) / indirect `JMP m16:32` (Group 5 `/5`)
  * Direct `CALL ptr16:32` (`66 9A`) / indirect `CALL m16:32` (Group 5 `/3`)
  * `RETF` (`66 CB`) / `RETF iw` (`66 CA`)
* Offset commits as IP16 (high half truncated — Vol. 2 JMP/CALL real-address
  note). CALL pushes a 6-byte frame (EIP32 then CS16). All forms **stay** in
  VM86 (`EFLAGS.VM` sticky); they are not a privileged exit.

## Unsupported (explicit)

* Privilege-changing call/task gates from VM86
* Treating a high offset as `#GP` (SDM truncates; we match that)
* VME

## Spec

Intel SDM Vol. 2 "JMP"/"CALL"/"RET" (far) + Ch. 2 (66H); Vol. 3 §20.1 /
§20.1.3; §3.4.2.

## Files

* `crates/x86-interpreter/src/lib.rs` — shared real/VM86 opsize-32 far path
  (comments clarified)
* `crates/x86-interpreter/tests/cpu_r11_vm86_far_opsize32.rs`
* `docs/cpu-r11-vm86-far-opsize32.md`
