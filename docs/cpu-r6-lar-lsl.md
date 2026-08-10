# Round 6 — LAR / LSL (`0F 02` / `0F 03`)

## Scope

Protected-mode `LAR` and `LSL`: load access rights or effective segment limit
into the destination register with ZF success/failure semantics.

## Behavior

Spec: Intel SDM Vol. 2 "LAR", "LSL".

- Real-address mode → `#UD`.
- Source selector is always 16-bit (`r16/m16`); destination follows operand size.
- Null selector, LDT (TI=1, unsupported here), out-of-GDT-limit, invalid type,
  or failed privilege check → `ZF=0`, destination unchanged.
- Success → `ZF=1` and load AR (LAR) or effective limit with G applied (LSL).
- Conforming code skips the DPL visibility check.
- LAR accepts system types `{1,2,3,4,5,9,B,C}`; LSL accepts `{1,2,3,9,B}`
  (call gates valid for LAR only). All code/data (`S=1`) are valid for both.

## Out of scope

- LDT resolution (TI=1 clears ZF)
- Long mode / REX.W
- `LOCK` `#UD`
- ARPL (still only real-mode `#UD` stand-in)

## Files

- `crates/x86-spec` — `LAR`/`LSL` metadata
- `crates/x86-decode` — decode coverage
- `crates/x86-interpreter` — execute path + ZF matrices
