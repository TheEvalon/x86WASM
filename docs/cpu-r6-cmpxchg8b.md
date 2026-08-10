# Round 6 — CMPXCHG8B (`0F C7 /1`)

## Scope

Decode and execute `CMPXCHG8B m64` in real-address and protected mode.

## Semantics

Spec: Intel SDM Vol. 2 "CMPXCHG8B/CMPXCHG16B".

- Compare `EDX:EAX` with the 64-bit memory destination.
- Equal: `ZF := 1`, store `ECX:EBX` to memory.
- Unequal: `ZF := 0`, load memory into `EDX:EAX`, write the old value back
  (locked read always pairs with a write).
- CF, PF, AF, SF, and OF are unaffected.
- Register destination (`mod=11`) raises `#UD`.
- `LOCK` may prefix the memory form; this tree is single-processor and does not
  model multi-processor bus locking.

## Honesty

- **CPUID.01H:EDX[8] (`CX8`) stays clear.** The instruction executes, but the
  feature bit is not advertised until the form is considered solid (and until
  related tooling expects it).
- Other Group 9 `/r` encodings remain unimplemented (`Unsupported`).
- `CMPXCHG16B` / REX.W is out of scope.

## Files

- `crates/x86-spec` — `GRP9` / `0F C7` metadata
- `crates/x86-decode` — mnemonic `CMPXCHG8B` for `/1`
- `crates/x86-interpreter` — execute path + tests
- `crates/machine-pc/tests/post_probe.rs` — known-absent stand-in moved from
  `0F C7` to `0F AE` (Group 15)
