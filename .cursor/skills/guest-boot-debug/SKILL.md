---
name: guest-boot-debug
description: Systematic debugging for firmware and guest OS boot failures in the emulator. Use when SeaBIOS/OVMF/FreeDOS/Linux/Windows hangs, triple-faults, or fails to reach an expected milestone.
---

# Guest boot debug

## Goal

Find the **first divergence** with evidence — do not spray speculative CPUID or device changes.

## Workflow

```text
Boot Debug:
- [ ] 1. Capture repro: image, machine profile, firmware, command
- [ ] 2. Note last known-good milestone (serial line, mode switch, etc.)
- [ ] 3. Collect: RIP/CS, CR0/CR3/CR4/EFER, mode, exception vector/error
- [ ] 4. Enable serial / port 0x402 / instruction trace around failure
- [ ] 5. Bisect: firmware vs CPU vs device vs timing
- [ ] 6. Compare against QEMU (same machine subset) when practical
- [ ] 7. Write a minimal failing test or ROM repro before fixing
- [ ] 8. Fix one root cause; re-run boot
```

## Hypotheses order

1. Wrong reset / memory map / firmware load
2. Missing or incorrect device register
3. Interrupt/timer routing
4. Mode switch / paging / descriptor bug
5. Unimplemented instruction actually executed (check #UD vs wrong decode)
6. CPUID lie causing guest path that needs missing features
7. SMP/APIC only if multicore is enabled

## Rules

- Do not "fix" boot by advertising CPUID features.
- Prefer a tiny assembler ROM or kvm-unit-style repro over full OS images when possible.
- Record findings in the PR/report: symptom → evidence → root cause → test added.
