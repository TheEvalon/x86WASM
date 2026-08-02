# Instruction metadata format

Decoder and interpreter consume declarative metadata from `x86-spec`. Generated tables (when present) must not be hand-edited — change metadata and regenerate.

## Schema (M1)

Each instruction entry includes:

- `mnemonic` — display name
- `opcode` — primary opcode byte(s)
- `encoding` — `none` / `modrm` / `imm8` / `imm16` / `imm32` / `rel8` / `rel16` / `opcode_reg`
- `operand_size` — `8` / `16` / `32` / `osz` (follows operand-size attribute)
- `notes` — SDM citation or subset limitation

Legacy prefixes recognized in M1: `0x66` (operand size), `0x67` (address size), segment overrides (`0x26`/`0x2E`/`0x36`/`0x3E`/`0x64`/`0x65`), `0xF0`/`0xF2`/`0xF3` (recorded; lock/rep semantics mostly unused in HELLO path).

## Decode pipeline

1. Parse legacy prefixes (stop at first non-prefix).
2. Read primary opcode.
3. If required, read ModRM (+ SIB / displacement).
4. Read immediates / relative offsets.
5. Reject truncated or overlong encodings with a structured error.

## Out of scope (M1)

- REX / VEX / EVEX
- Full 3DNow / x87 / SSE maps
- XED differential harness (wrapper stub only)
