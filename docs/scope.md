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

Milestone 0–1 exit (HELLO ROM) is met on `main`. Active work is **Milestone 2**: real-mode foundation → interrupts/exceptions → legacy PC devices → SeaBIOS/FreeDOS. Track checkboxes in `plan.md` §21 Milestone 2.

**Not yet:** protected mode, paging, PIC/PIT, VGA/IDE, SeaBIOS boot, Windows, JIT, net, audio, SMP.
