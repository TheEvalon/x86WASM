# Privilege-changing interrupt/trap gate delivery (Milestone 2, round 5)

When a gate's target code segment has `DPL < CPL`, delivery switches stacks
through the current TSS before pushing the exception/interrupt frame.

## Authority

| Rule | Section |
|---|---|
| Stack switch; push outer `SS:ESP` then `EFLAGS/CS/EIP` [/error] | Vol. 3 §6.12.1 Figure 6-5 |
| `ESP0`/`SS0` (and level 1/2) offsets in a 32-bit TSS | Vol. 3 §7.2.1 |
| Inner-stack / descriptor-table accesses are supervisor | Vol. 3 §4.6.1 |
| Error-code width follows the gate (word vs dword) | Vol. 3 §6.13 / §6.12.1 |

No implementation from another emulator was read or copied.

## Supported

* Nonconforming target with `DPL < CPL`: load `SSn:ESPn` from the busy/available
  32-bit TSS cached in `TR`, validate the new SS at the inner CPL, push outer
  `SS:ESP` then the ordinary frame on the new stack, commit `SS`/`ESP`/`CS`/`EIP`.
* Same-CPL path unchanged (`DPL == CPL`).
* 16-bit and 32-bit interrupt/trap gates; atomic rollback on stack failure.

## Not supported

* Task gates, VM86 delivery, conforming privilege shortcuts, 16-bit TSS,
  hardware task switch, nested `#DF`/triple-fault (next slices cover `#DF`
  and outer `IRET`).
