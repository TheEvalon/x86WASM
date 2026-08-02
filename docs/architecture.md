# Architecture

Crate boundaries and execution model for x86WASM. Product detail: `plan.md` §7–§8.

## Layers

```text
web/ (Vite + TS)  →  emulator-web (Wasm)  →  machine-pc
                                              ├─ x86-interpreter  → x86-decode → x86-spec
                                              ├─ x86-core / x86-mmu / x86-ir
                                              └─ devices / firmware-interface
emulator-cli (native) ────────────────────────┘
```

## Rules

- **Interpreter is truth.** JIT (later) must match interpreter architectural state.
- **No browser APIs** in `x86-core`, `x86-decode`, `x86-mmu`, `x86-ir`, `x86-interpreter`, or `devices`.
- **64-bit-capable CPU state** from the first commit (`u64` GPRs and addresses).
- **Truthful CPUID** — never advertise unimplemented features.
- Devices attach through port I/O and MMIO buses owned by `machine-pc`.

## Crates (Milestone 0/1)

| Crate | Role |
|---|---|
| `x86-spec` | Declarative instruction metadata (schema + subset tables) |
| `x86-decode` | Prefix / opcode / ModRM decode |
| `x86-core` | `CpuState`, reset, compare helpers |
| `x86-mmu` | Address translation stubs (identity / real-mode base+offset for M1) |
| `x86-ir` | Typed IR placeholder (JIT later) |
| `x86-interpreter` | Reference execution |
| `machine-pc` | Wire CPU, memory, buses, devices |
| `devices` | COM1, debug port `0x402`, future PIC/PIT/… |
| `firmware-interface` | Firmware load hooks (stub in M1) |
| `emulator-cli` | Native runner |
| `emulator-web` | Wasm exports for the browser worker |

JIT crate (`x86-jit-wasm`) is deferred until Milestone 4.
