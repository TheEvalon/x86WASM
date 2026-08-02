# Scope

Summary for agents. Full detail: repository root `plan.md`.

## Goals (required)

- Full-system x86 PC emulator in the browser (Rust → Wasm)
- Real / protected / compatibility / long mode path toward Core 2-era + Win10 x64
- SeaBIOS then OVMF; interpreter first, Wasm JIT later
- Clean TS API + v86 compatibility adapter for oses.ioblako.com

## Non-goals (v1)

- Cycle-accurate Core 2, VT-x, AVX*, Windows 11, TPM 2.0, Secure Boot, 3D GPU
- Reimplementing BIOS/UEFI from scratch
- Copying code from other emulators

## Current build focus

Milestone 0–1: docs/ADRs, workspace, CPU state, buses, decoder framework, minimal interpreter, serial debug ROM.

**Not yet:** VGA/IDE/Windows/JIT/net/audio/SMP.
