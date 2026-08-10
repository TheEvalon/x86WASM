# Round 7 — GDT 32-bit call-gate far CALL

## Scope

Protected-mode far `CALL` (`9A` / `FF /3`) through a present **32-bit call
gate** (`type=0xC`) in the GDT, with **parameter count 0**.

## Behavior

* Gate DPL: `CPL ≤ DPL` and `RPL ≤ DPL`; not-present → `#NP`.
* Target code: executable, `DPL ≤ CPL`; conforming never changes CPL;
  nonconforming with `DPL < CPL` switches stack via TSS `SSn:ESPn`.
* Same-CPL: push dword `CS` then `EIP`, enter gate offset.
* Privilege change: supervisor writes of `SS`/`ESP`/`CS`/`EIP` on the inner
  stack (Vol. 3 Figure 5-9), then load CS with RPL = new CPL.
* Instruction far-pointer offset is ignored; the gate supplies the offset.

## Out of scope

* 16-bit call gates (`type=4`)
* Non-zero parameter count (argument copy)
* LDT-resident call gates
* Far `JMP` through a call gate
* Nested-task `CALL` to TSS / task gate

## Spec

Intel SDM Vol. 2 "CALL"; Vol. 3 §5.8.2 (Figures 5-8 / 5-9), §7.2.1.

## Files

* `crates/x86-interpreter/src/lib.rs`
* `crates/x86-interpreter/tests/cpu_r7_call_gate.rs`
* `docs/cpu-r7-call-gate.md`
