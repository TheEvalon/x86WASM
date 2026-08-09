# Two-byte `0F` opcode map (Milestone 2, round 2)

Scope: what the `x86-decode` / `x86-interpreter` two-byte map implements after
the round-2 CPU slices, and — just as importantly — what it does not.

Authority: Intel SDM Vol. 2 (instruction reference and Appendix A opcode maps),
Vol. 3 Ch. 5–6. No behavior here is taken from another emulator.

Before this round the map held only `0F 01` (Group 7), `0F 06` (`CLTS`),
`0F 20`/`0F 22` (`MOV` to/from `CR0`), and `0F AF` (`IMUL`).

## Shared rules

Every form added in this round obeys the round-1 size rules:

- The operand-size and address-size attributes come from the code segment's
  cached `D` bit; `0x66` and `0x67` select the *other* size, so they invert
  under `D=1` (SDM Vol. 1 §3.6 Table 3-4; Vol. 3 §3.4.5).
- Instruction-pointer commits go through the shared `set_current_ip`, which
  writes `IP` only under `D=0` and the full `EIP` under `D=1`.
- Byte register operands use the shared legacy `AL..BH` accessors, so `mod=11`
  encodings 4–7 address the high bytes.
- Memory operands go through the shared effective-address helpers, so cached
  segment limits, the `#GP`/`#SS` split, and 32-bit ModR/M + SIB addressing
  behave exactly as they do in the primary map.

## Slice 1 — `Jcc` near (`0F 80`–`0F 8F`) and `SETcc` (`0F 90`–`0F 9F`)

Spec: SDM Vol. 2 "Jcc—Jump if Condition Is Met", "SETcc—Set Byte on
Condition", Appendix B (condition-code encodings); Appendix A opcode map 2.

All sixteen condition codes are evaluated by one shared helper keyed on the low
nibble, so the short `70`+cc form, the near `0F 80`+cc form, and `0F 90`+cc
`SETcc` cannot disagree.

Supported:

- Near `Jcc` with a rel16 displacement (16-bit operand size), which clears
  `EIP[31:16]` on a taken branch, and with a rel32 displacement (32-bit operand
  size). Not-taken branches fall through to the sequential next `EIP`. Neither
  outcome writes flags.
- `SETcc r/m8` with a register destination (including `AH`/`CH`/`DH`/`BH`) and
  with a memory destination in both 16-bit and 32-bit addressing. Exactly one
  byte is written, `1` or `0`, and no flag is modified. `ModR/M.reg` is not
  used, so any value in that field decodes and executes.

Not supported:

- The `CS`-limit `#GP` for a near branch is still detected on the next
  instruction fetch rather than at branch time.
- A 32-bit operand-size near `Jcc` executed from a `D=0` code segment commits
  only `IP`, because `set_current_ip` owns that window rule for the whole
  interpreter.
- 64-bit mode (`rel32` sign-extended into `RIP`) and the branch hint prefixes
  `2E`/`3E`.

## Slice 2 — extension moves and segment loads

Spec: SDM Vol. 2 "MOVZX—Move with Zero-Extend", "MOVSX—Move with
Sign-Extension", "PUSH", "POP", "LDS/LES/LFS/LGS/LSS—Load Far Pointer";
Vol. 3 §§3.4.2–3.4.5, 5.4.1, 6.8.3, 6.15.

Supported:

- `MOVZX`/`MOVSX` (`0F B6`/`B7`/`BE`/`BF`) in all eight source/destination
  width combinations. The opcode fixes the source width and the operand-size
  attribute fixes the destination width, so a word source with a 16-bit
  operand size is an ordinary word move. A 16-bit destination leaves the upper
  half of the 32-bit register untouched. Byte sources reach memory and the
  legacy `AL..BH` registers. No flags are written.
- `PUSH FS`/`GS` and `POP FS`/`GS` (`0F A0`/`A1`/`A8`/`A9`). The stack slot is
  a word under a 16-bit operand size and a doubleword holding the zero-extended
  selector under a 32-bit one; the stack-pointer width itself still follows
  `SS.B`. In protected mode `POP FS`/`GS` validate the selector through the
  shared DS/ES data-descriptor path (null selectors allowed, clearing the
  cache) before either the stack pointer or the cache commits.
- `LSS`/`LFS`/`LGS` (`0F B2`/`B4`/`B5`) with `m16:16` and `m16:32` pointers.
  They reuse the same descriptor helpers as `LDS`/`LES`, so `SS` gets the
  stack-segment rules (null selector `#GP(0)`, writable ring-matched data
  required, `P=0` reported as `#SS`) and `FS`/`GS` get the DS/ES data rules.
  Nothing commits until the whole pointer is read and the descriptor validates.
  `LSS` arms the same maskable-interrupt shadow as `MOV SS`/`POP SS`;
  `LFS`/`LGS` do not. The register form (`mod=11`) is `#UD`.

Not supported:

- The primary-map `POP ES`/`SS`/`DS` (`07`/`17`/`1F`) and `PUSH ES`/`CS`/`SS`/`DS`
  (`06`/`0E`/`16`/`1E`) still use a 16-bit stack slot regardless of the
  operand-size attribute. That is pre-existing behavior outside this slice, but
  it is now inconsistent with `0F A0`/`A1`/`A8`/`A9` and should be a round-3
  item.
- `MOVSXD` and the REX.W 64-bit destinations.
- Loading `SS` from the LDT, or any privilege-level change.
