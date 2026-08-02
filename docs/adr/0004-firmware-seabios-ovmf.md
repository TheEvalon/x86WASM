# ADR-0004: Firmware — SeaBIOS then OVMF

- Status: Accepted
- Date: 2026-08-02

## Context

Firmware must be real (not reimplemented). Licensing and build provenance matter.

## Decision

1. **SeaBIOS** (+ SeaVGABIOS as needed) for legacy BIOS boot and FreeDOS/Linux32 path.
2. **OVMF** later for UEFI / Windows x64 path.
3. Firmware binaries live under `firmware/` with scripts, licenses, and `third_party/NOTICE` entries — never pasted into MIT/Apache crates as GPL source.

## Consequences

Machine model tracks QEMU/SeaBIOS expectations. Custom reset ROMs are allowed for lab tests (HELLO ROM) without replacing production firmware goals.
