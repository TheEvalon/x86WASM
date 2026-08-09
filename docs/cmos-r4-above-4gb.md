# CMOS memory above 4 GB (`5Bh`–`5Dh`)

## Status: de-facto standard, not silicon

This is the one place in the CMOS model whose source is not a datasheet or
Ralf Brown's Interrupt List. `docs/adr/0006-cmos-above-4gb-memory.md` decided to
adopt it anyway and required the status to be stated wherever it appears, so it
is stated here, in the constant doc comments, and in the test module header.

The MC146818 register file ends at `0Dh`. Everything above it is general CMOS
RAM whose meaning is assigned by whoever wrote the BIOS. Bochs introduced the
`5Bh`/`5Ch`/`5Dh` convention for reporting memory above 4 GB, QEMU follows it,
and SeaBIOS reads exactly those indices — but no chipset or RTC datasheet
defines them, and none ever will, because this is a software convention that
became load-bearing.

The ADR's test applies: the encoding is a fact two implementations must agree on
to interoperate, discoverable from documentation of the convention. That does
not license reading anyone's implementation of it, and none was read.

## Encoding

| Index | Field |
|---|---|
| `5Bh` | bits 7:0 of the count |
| `5Ch` | bits 15:8 |
| `5Dh` | bits 23:16 |

The count is whole 64 KiB units of memory above 4 GB — the same unit as the
`34h`/`35h` pair, so the two ranges add up. A partial unit is dropped rather
than rounded up into memory that does not exist, and the 24-bit field saturates
at `FFFFFFh` rather than wrapping.

## The range split this introduces

The memory-size registers now partition the address space rather than overlap:

| Registers | Range | Unit |
|---|---|---|
| `15h`/`16h` | 0 to 640 KB | KB |
| `17h`/`18h`, `30h`/`31h` | 1 MB to 16 MB | KB, capped at `3C00h` = 15 MB |
| `34h`/`35h` | 16 MB to 4 GB | 64 KB blocks, capped at `FF00h` |
| `5Bh`–`5Dh` | above 4 GB | 64 KiB units |

`34h`/`35h` previously saturated at `FFFFh`. That was the best a model with
nowhere to put the remainder could do, but it is wrong once `5Bh`–`5Dh` exists:
`FFFFh` blocks past 16 MB reaches 64 KiB beyond 4 GB, so the last block would be
counted twice and everything above it lost. The cap is now `FF00h`, which makes
`16 MB + FF00h × 64 KiB` exactly 4 GB — asserted at compile time next to the
constant.

## Durability

`5Bh`–`5Dh` are battery backed alongside `30h`/`31h` and `34h`/`35h`
(`CmosRtc::is_battery_backed`). They are memory-size bytes with the same
lifetime: a reset must not erase the machine's memory map. They remain ordinary
read/write CMOS RAM afterwards, so a guest can overwrite them.

## Not exercised by a machine yet

Nothing in the tree configures more than 4 GB of guest RAM, so this path is
covered by tests rather than by a running machine. That was the ADR's reason for
deferring the code in round 2; it is implemented now so the encoding is settled
and tested before a large-memory configuration needs it, not after.
