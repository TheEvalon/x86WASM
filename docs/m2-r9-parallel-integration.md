# Milestone 2 Round 9 parallel integration

Branch: `merge/m2-r9-parallel-16` (base parallel R8 tip `dbf1dcb`; verified R8 tip `f9af9d9` not yet joined — see note).

Four lanes merged in order: platform-post → display-fw → boot-guest → cpu-vm86.

## Landed (16 slices)

| Lane | Tip | Highlights | Docs |
|---|---|---|---|
| platform-post | `172430f` | PM_TMR freerun on `tick_pit`; APM SMI halt-wake; 8042 scancode queue + host INT 16h AH=00/01; port61 refresh/speaker AND | `docs/platform-r9-*`, `docs/apm-r9-*`, `docs/kbd-r9-*`, `docs/pit-r9-*` |
| display-fw | `387e554` | VBE PhysBasePtr honesty; host INT 10h AH=00/0Eh; option-ROM POST scan; SeaVGABIOS Linux/WSL smoke | `docs/vga-r9-*`, `docs/firmware-r9-*` |
| boot-guest | `80e0ce2` | INT 13h AH=43h; floppy AH=02/03; FreeDOS-like + Linux serial measure | `docs/storage-r9-*`, `docs/boot-r9-*` |
| cpu-vm86 | `b1c46a5` | VM86 IRETD enter; CLI/STI IOPL; PUSHF/POPF IOPL; VM86 IRET #GP + monitor exit | `docs/cpu-r9-*` |

## Merge notes

- R9 worktrees were based on parallel R8 merge `dbf1dcb` (same lane content as `f9af9d9`, different merge commit graph).
- Joining verified tip `f9af9d9` deferred: restores INT 13h **AH=48h** EDD params that exist on `f9af9d9` but conflict with R9 floppy/AH=43 in `int13.rs`. Track as next-round join.

## Honesty that survives

- Host INT 10h/13h/16h are not SeaBIOS bodies.
- No guest LFB / full VGA BIOS; SeaVGABIOS Windows build still infeasible.
- FreeDOS/Linux measure harnesses do **not** claim prompt/shell (M2 exits open).
- VM86 interrupt-from-VM86 / VME/PVI still out.
- AH=48h may be missing until `f9af9d9` join (advertise only CX packet bit on this tip).
