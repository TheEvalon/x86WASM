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
