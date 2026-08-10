# Milestone 2 Round 9 parallel integration

Branch: `merge/m2-r9-parallel-16` (base `merge/m2-r8-parallel-16` / `dbf1dcb`).

Four lanes merged in order: platform-post → display-fw → boot-guest → cpu-vm86.

## Landed (16 slices)

| Lane | Branch tip | Highlights | Docs |
|---|---|---|---|
| platform-post | `172430f` | PM_TMR freerun on `tick_pit`; APM SMI halt-wake deepen; 8042 scancode queue + host INT 16h AH=00/01; port61 refresh/speaker AND | `docs/platform-r9-*`, `docs/apm-r9-*`, `docs/kbd-r9-*`, `docs/pit-r9-*` |
| display-fw | `387e554` | VBE PhysBasePtr honesty; host INT 10h AH=00/0Eh; option-ROM POST scan invoke; SeaVGABIOS Linux/WSL smoke | `docs/vga-r9-*`, `docs/firmware-r9-*` |
| boot-guest | `80e0ce2` | INT 13h AH=43h; floppy AH=02/03; FreeDOS-like measure; Linux serial measure harness | `docs/storage-r9-*`, `docs/boot-r9-*` |
| cpu-vm86 | `b1c46a5` | VM86 enter via IRETD; CLI/STI IOPL #GP; PUSHF/POPF IOPL; VM86 IRET #GP + monitor exit | `docs/cpu-r9-*` |

## R8 tip join (AH=48 / EDD)

R9 was gated on R8 lineage tip `dbf1dcb`, which predated INT 13h AH=48h.
Alternate R8 tip `merge/m2-r8-parallel-16` @ `f9af9d9` (boot tip `deb030a`)
carried `5ce989d` (AH=48h EDD params + AH=08/41 deepen) + rustfmt follow-up.

Joined onto this branch by cherry-pick (preferred over merging the alternate R8 tip
to avoid replaying parallel merge commits):

- `13d2870` ← cherry-pick of `5ce989d` (resolved with R9 AH=43h / floppy)
- `51944fd` ← cherry-pick of `deb030a` (rustfmt)

Host INT 13h HD subset is now AH=00/02/03/08/41/42/43/48. See
`docs/storage-r8-int13-ext.md` and `docs/storage-r8-int13-extensions.md`.

## Merge notes

- Auto-merged without manual conflict resolution (`machine-pc/lib.rs`, `devices/lib.rs`).
- Host BIOS stubs (INT 10h/13h/16h) remain host-installed IVT helpers — not SeaBIOS bodies.
- CPU VM86 is enter + IOPL-sensitive subset; interrupt-from-VM86 frame push still out.
- Quality gate on merge tip: `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` green.

## Honesty that survives

- No real SMM; APM is deepened stub only.
- No guest LFB / full VGA BIOS / VBE `4Fxx`.
- SeaVGABIOS Windows native build still infeasible; Linux/WSL smoke documented.
- FreeDOS/Linux measure harnesses do **not** claim prompt/shell (M2 exits still open).
- VM86→CPL0 interrupt delivery, VME/PVI, far control transfers while `VM=1` still out.
- SeaBIOS POST still incomplete.
- AH=48h is Phoenix EDD v1.x 26-byte subset only (no EDD 2.0/3.0 path/DPI; no removable lock).
