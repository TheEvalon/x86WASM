# ADR-0001: Classic PC machine model (v1)

- Status: Accepted
- Date: 2026-08-02

## Context

Guests and firmware expect a QEMU/SeaBIOS-compatible classic PC subset, not a novel platform.

## Decision

Implement **PC v1**: legacy BIOS path first (SeaBIOS), IDE/ATAPI, VGA text then graphics, PIC/PIT/RTC, PCI config as needed. Document ports/MMIO in `docs/machine-model-pc-v1.md`. UEFI/OVMF is a later second firmware path on the same machine family.

## Consequences

Device work prioritizes SeaBIOS bring-up. Exotic chipsets and modern platform devices stay out of scope until explicitly planned.
