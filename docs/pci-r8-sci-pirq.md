# ACPI SCI_EN honesty and optional SCI→PIRQ stub — Milestone 2 Round 8

## Why

Round 5/6 already compute `acpi_sci_asserted()` as
`SCI_EN && (PM1_STS & PM1_EN & SCI_mask)`. Firmware still has no FADT
`SCI_INT` wire onto the interrupt controller. This slice keeps the SCI level
**honest** and adds an **optional** host soft-wire onto a software PIRQ pin so
tests (and later machine glue) can exercise PIRQRC → DualPic without inventing
an interrupt-link device.

## Spec

- ACPI Specification §4.8.1 — SCI generation from enabled PM1 status bits while
  `SCI_EN` is set; when `SCI_EN` is clear, PM1 events take the SMI path (this
  tree still has no SMI delivery).
- Intel 82371SB — PIRQRC[A:D] at ISA config `0x60`–`0x63`; software
  [`PciConfig::set_pirq_line`] / `sync_pirq_to_pic`.

## Model

| Piece | Behaviour |
|---|---|
| `acpi_sci_asserted` | Level only; `SCI_EN=0` ⇒ false even if STS&EN match |
| `sync_acpi_sci_to_pirq(pirq)` | Sets software PIRQA–D high iff SCI is asserted |
| PIC delivery | Still requires `sync_pirq_to_pic` and an enabled PIRQRC route |

Classic PC platforms usually route ACPI SCI to **IRQ9** via an interrupt-link
/_PRT path, **not** through PIRQRC. This soft-wire is therefore a documented
stand-in until FADT/`SCI_INT` exists — not a claim that PIIX wires SCI to PIRQ#.

`Machine::sync_acpi_sci_to_pirq` mirrors the level and immediately syncs PIC.

## Not implemented

- FADT `SCI_INT`, interrupt-link devices, `_PRT`
- Automatic SCI→PIC on every PM1 write (host must call the sync helper)
- SMI# when `SCI_EN=0`
