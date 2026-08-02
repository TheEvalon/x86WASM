---
name: implement-instruction
description: Implements a bounded x86 instruction slice (metadata, decode, interpreter semantics, tests). Use when adding or fixing opcodes, flags, or CPU instruction behavior — not devices, JIT-only work, or full ISA ports.
---

# Implement instruction

## Preconditions

- Slice is named with explicit opcodes/forms (e.g. `MOV 0x88–0x8B`, `ADD r/m32,r32`).
- Spec refs identified (Intel SDM volumes/sections or `docs/`).

## Workflow

Copy and track:

```text
Instruction Progress:
- [ ] 1. Failing tests first
- [ ] 2. Metadata (if new encoding)
- [ ] 3. Decoder coverage
- [ ] 4. Interpreter semantics
- [ ] 5. Flags / exceptions / modes
- [ ] 6. Oracle diff if available
- [ ] 7. quality-gate
```

### 1. Failing tests first

Cover at minimum what applies:

- Result registers/memory
- Flags written
- Exceptions (#UD, #GP, #PF, #SS, etc.)
- Operand widths
- Memory forms
- Relevant modes (real / protected / compat / long)

### 2. Metadata

- Edit declarative defs in `crates/x86-spec` only.
- Regenerate; never hand-edit generated tables.

### 3. Decoder

- Support only encodings in scope (e.g. no REX if slice says so).
- Truncated and invalid encodings must fail cleanly with tests.

### 4. Interpreter

- Implement semantics via shared helpers for flags and writes.
- Precise exceptions: no architecturally illegal partial updates.
- Do **not** implement JIT in the same slice unless explicitly requested.

### 5. Finish

- Run `quality-gate`.
- Report unsupported cases remaining.
- Do not flip CPUID bits for unfinished feature sets.

## Example acceptance block

```text
ADD r/m32, r32:
- reg and mem destinations
- CF PF AF ZF SF OF correct
- cross-page mem access
- segment-limit / read-only PF before modify
- EIP updates only on success
- native oracle differential where practical
- no JIT in this issue
```
