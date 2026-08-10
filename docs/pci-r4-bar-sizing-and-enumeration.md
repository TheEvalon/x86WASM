# PCI BAR sizing and the enumeration surface

What a firmware `pci_probe_devices` scan sees on this machine, and how a Base
Address Register answers the all-ones sizing protocol.

## Authority

PCI Local Bus Specification Revision 3.0:

- §6.1 Figure 6-1 "Type 00h Configuration Space Header" — register layout.
- §6.2.1 "Device Identification" — Vendor ID, Device ID, Revision ID, Class
  Code, Header Type; "FFFFh is an invalid value for Vendor ID"; Header Type
  bit 7 "identifies a multi-function device".
- §6.2.5.1 "Base Addresses" — the encoding bits, the writable-bit/hardwired-bit
  model, and the sizing calculation.
- §6.2.5.2 "Expansion ROM Base Address Register".
- §6.2.4 "Miscellaneous Registers" — Interrupt Line, Interrupt Pin, BIST,
  Capabilities Pointer.
- §6.7 "Capabilities List" — the Capabilities Pointer is meaningful only when
  Status bit 4 is set.

Intel 440FX PCIset 82441FX (PMC) and Intel 82371SB/82371AB (PIIX3/PIIX4)
datasheets for the per-function register defaults and reserved ranges.

## The enumeration surface

A scan walks bus 0, devices 0-31, functions 0-7. It reads the Vendor ID first;
`0xFFFF` means "nothing here". It reads function 0's Header Type next, and only
looks at functions 1-7 when bit 7 is set. This machine answers on five of those
256 addresses:

| Address | Vendor:Device | Class | Prog IF | Header Type |
|---|---|---|---|---|
| `00:00.0` | `8086:1237` i440FX | `0600` host bridge | `00` | `00` |
| `00:01.0` | `8086:7000` PIIX3 ISA | `0601` ISA bridge | `00` | `80` |
| `00:01.1` | `8086:7010` PIIX3 IDE | `0101` IDE | `80` | `00` |
| `00:01.2` | `8086:7020` PIIX3 USB | `0C03` USB | `00` UHCI | `00` |
| `00:01.3` | `8086:7113` PIIX4 ACPI | `0680` other bridge | `00` | `00` |

The multi-function bit on `00:01.0` is load-bearing: without it firmware never
reads `00:01.1`-`00:01.3` at all, and the IDE, USB and ACPI functions become
invisible no matter how correct their registers are.

### Read-only bytes

Before this slice, every header byte outside the identity and class registers
was ordinary read/write storage. A scan that probes a register by writing to it
— which is exactly what BAR sizing does, and what some firmware does to
Capabilities Pointer and BIST — would have changed what the next reader saw.
These are now read-only:

| Range | Register | Why zero is the honest answer |
|---|---|---|
| `0x00`-`0x03`, `0x08`-`0x0B`, `0x0E` | identity, class, header type | §6.2.1 read-only |
| `0x0F` | BIST | §6.2.4: no BIST here, so it must return 0 |
| `0x28`-`0x2B` | CardBus CIS Pointer | unused |
| `0x2C`-`0x2F` | Subsystem Vendor ID / Subsystem ID | this machine assigns none |
| `0x34` | Capabilities Pointer | Status bit 4 is clear on every function |
| `0x38`-`0x3B` | reserved | Figure 6-1 |
| `0x3D` | Interrupt Pin | no function drives INTA#-INTD# |
| `0x3E`-`0x3F` | Min_Gnt / Max_Lat | no bus-timing requirement stated |

`0x3C` Interrupt Line stays read/write: §6.2.4 makes it the byte POST fills in
with the routed IRQ, and nothing reads it back here.

### Qualified Prog IF advertising

`00:01.1`'s programming interface byte is `PCI_PROG_IF_IDE_BUS_MASTER`
(`0x80`), the bus-master IDE bit. That matches what this tree actually exposes:

- the BMIDE I/O BAR (BMIBA) decode when Command.IO is set;
- host-called primary-channel PRD walkers (`start_bm_read` / `start_bm_write`);
- Command.BusMaster as the gate for those host helpers.

It does **not** claim a complete ATA DMA engine: a guest write to BMICOM.SSBM is
ordinary register store/readback and does not start a transfer, and no ATA
READ DMA / WRITE DMA command arms the BMIDE block. Clearing the Prog IF bit
would make a PIIX3 device ID look master-incapable, which is a stranger lie for
firmware that keys on `8086:7010`. The gap is therefore documented here and in
`docs/pci-bmide-prd-directions.md` rather than hidden behind a cleared bit.

## The sizing protocol

§6.2.5.1: "Software saves the original value of the Base Address register,
writes 0 FFFF FFFFh to the register, then reads it back. Size calculation can
be done from the 32-bit value read by first clearing encoding information bits
(bit 0 for I/O, bits 0-3 for memory), inverting all 32 bits (logical NOT), then
incrementing by 1."

That works because a device "would build the top bits of the address register,
hardwiring the other bits to 0". So a register of size *S* has writable mask
`!(S - 1)`, hardwired zeros below it, and read-only encoding bits underneath.
`PciBarSpec` is exactly those three facts, and every BAR write goes through
`PciBarSpec::readback`, so a byte, word, or dword write all obey the same rule.

## What this machine implements

| Function | Offset | Kind | Size | Sizing read-back |
|---|---|---|---|---|
| `00:01.1` PIIX IDE | `0x20` (BAR4) | I/O | 16 B | `0xFFFFFFF1` |
| `00:01.2` PIIX USB UHCI | `0x20` (BAR4) | I/O | 32 B | `0xFFFFFFE1` |

Everything else is read-only zero: all six BARs on `00:00.0`, `00:01.0` and
`00:01.3`, the five unused BAR slots on the IDE and USB functions, and the
Expansion ROM register at `0x30` on every function. Before this slice those
positions were ordinary read/write storage, so a sizing pass read back the
`0xFFFFFFFF` it had just written and would have concluded that the host bridge
requested a 4 GB region.

Both implemented registers read back their I/O-space bit **from reset**, before
firmware programs anything. That matters: firmware reads the register first to
decide whether it is looking at an I/O or a memory region, so a BAR that read
all-zero at reset would be sized and assigned as memory. Intel 82371SB gives
BMIBA the matching default value `00000001h`.

## Model choice: the writable mask runs to bit 31

The PIIX datasheets describe bits 31:16 of BMIBA and of the UHCI I/O base as
Reserved, because that silicon decodes only the 16-bit x86 I/O space. Read
literally, an all-ones write would read back `0x0000FFF1`, and §6.2.5.1's
calculation on that value yields `0xFFFF0010` — not a region size at all.

This model keeps the writable bits running to bit 31 so the documented
calculation produces the documented answer, and handles the 16-bit decode limit
where it actually belongs: `bmide_io_base` and `uhci_io_base` refuse to decode a
base at or above `0x10000`. A machine cannot address such a port with `IN`/`OUT`
anyway (Intel SDM Vol. 1 §18.3 — the I/O address space is 64 KB).

This is a **model choice**, not a datasheet reading, and it is recorded here for
the same reason ADR-0006 records the CMOS `5Bh` encoding: the interoperable
behavior and the vendor document disagree, and pretending otherwise would hide
the disagreement rather than settle it.

## Unsupported (explicit)

- Memory BARs. `PciBarSpec` can encode the type and prefetchable bits, but no
  function here exposes one, so no memory window is ever decoded from a BAR.
- 64-bit BARs (type `10b`), which would consume the following register.
- Expansion ROMs: the register is read-only zero, and no option ROM exists.
- PIIX ACPI PMBASE at `0x40` is a device-specific base register outside the
  Type 0 BAR block. No sizing protocol runs on it, and unlike the two real BARs
  it stays fully zero at reset rather than taking the datasheet's `00000001h`
  default — so nothing decodes at I/O port 0 before firmware programs it. That
  difference is deliberate and is the one place this file departs from the
  "hardwired bit 0" rule above.
- Base Address Register *decode* beyond the two I/O windows already stubbed
  (BMIDE registers, UHCI registers): programming a BAR does not create any new
  MMIO region.
