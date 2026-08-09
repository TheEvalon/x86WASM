# Outer-privilege `IRET` / `IRETD` (Milestone 2, round 5)

Return from a ring-0 interrupt/trap handler to a less-privileged code segment,
restoring the outer `SS:ESP` saved by privilege-changing delivery.

## Authority

| Rule | Section |
|---|---|
| Outer-level return pops `EIP/CS/EFLAGS` then `ESP/SS` | Vol. 2 IRET Operation |
| Return CS.RPL becomes the new CPL; SS matches that CPL | Vol. 2 IRET; Vol. 3 §5.5 |
| Stack switch frame layout | Vol. 3 §6.12.1 Figure 6-5 |

No implementation from another emulator was read or copied.

## Supported

* `IRET`/`IRETD` at CPL 0 with return CS.RPL > 0: validate nonconforming
  return CS (`DPL == RPL`), load matching writable SS, restore outer ESP,
  drop CPL to RPL, restore flags from the image.
* Same-CPL ring-0 `IRET`/`IRETD` unchanged.

## Not supported

* `IRET` executed at CPL > 0, conforming return CS, `NT=1` task returns,
  VM86 returns, LDT selectors.
