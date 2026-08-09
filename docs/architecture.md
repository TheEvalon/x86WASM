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
| `devices` | COM1/COM2 16550 stubs, debug port `0x402`, PIC/PIT/… |
| `firmware-interface` | Firmware load hooks (stub in M1) |
| `emulator-cli` | Native runner |
| `emulator-web` | Wasm exports for the browser worker |

## Milestone 2 execution boundary

Real mode remains the complete M2 CPU foundation. With `CR0.PE=1`, the
interpreter now also has a bounded 16-bit protected-mode path:

- GDT-backed `MOV`/`POP` segment loads and `LDS`/`LES` validate the complete
  descriptor before atomically updating visible and hidden segment state.
- Direct far `JMP ptr16:16` / `JMP m16:16` can load a same-level,
  nonconforming ring-0 `D=0` code segment.
- Architectural faults, software `INT`/`INT3`/taken `INTO`, NMI, and maskable
  IRQs can enter same-CPL 16-bit IDT interrupt/trap gates. Interrupt gates clear
  IF; trap gates preserve it; applicable faults push selector/error-code words.
- Same-CPL ring-0 `IRET16` validates its full frame and target descriptor before
  commit. Successful `MOV SS` / `POP SS` blocks maskable interrupts through the
  following instruction boundary without blocking NMI.
- Failed descriptor, frame, or stack validation is atomic. A nested delivery
  failure is reported deterministically; double-fault/triple-fault synthesis is
  not implemented.

`SegmentReg.flags` preserves the descriptor access byte plus AVL/L/D-B/G.
That state does **not** imply default-32 execution support: default-32 code or
stacks, 32-bit gates/IRETD, privilege-level stack switching, outer-level
returns, call gates/tasks, LDT/TSS, and paging remain unsupported.

The native CLI steps the machine directly so an execution failure retains the
original `MachineError` and reports completed steps, `CS:IP`, full RIP, linear
PC, and an eight-byte wrapping opcode window. This is diagnostic context only;
it does not change interpreter semantics.

JIT crate (`x86-jit-wasm`) is deferred until Milestone 4.
