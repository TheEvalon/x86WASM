# ADR-0006: CMOS `5Bh`–`5Dh` above-4 GB memory is a de-facto standard, not silicon

- Status: Accepted
- Date: 2026-08-09

## Context

The round-2 CMOS work populated the memory-size registers a BIOS reads to build
its memory map: `15h`/`16h` base memory in KB, `17h`/`18h` and `30h`/`31h`
extended memory in KB, and `34h`/`35h` memory above 16 MB in 64 KB blocks. All
four pairs are documented in Ralf Brown's Interrupt List and are now listed in
`docs/sources.md`.

That set stops at 4 GB. A machine with more memory than that has nothing to
report it in, and the CMOS implementation left `5Bh`–`5Dh` at zero rather than
inventing an encoding — the right call at the time, and the reason this ADR
exists rather than a code change.

The problem is that `5Bh`–`5Dh` *is* what firmware reads. Bochs introduced the
convention and QEMU follows it: three bytes at `5Bh` (low), `5Ch` (middle) and
`5Dh` (high) holding the amount of memory above 4 GB in 64 KiB units. SeaBIOS
reads exactly those indices. But no chipset or RTC datasheet defines them. The
MC146818 register file ends at `0Dh`; everything above it is general CMOS RAM
whose meaning is assigned by whoever wrote the BIOS. There is no authoritative
register map to cite, and there never will be one, because this is a software
convention that became load-bearing.

The repo rule is "never invent behavior; cite a spec". Applied literally, that
rule forbids implementing `5Bh`–`5Dh` at all — which would also mean this
emulator can never honestly describe more than 4 GB of guest RAM to the
firmware it is being built to boot.

## Decision

**Adopt the Bochs/QEMU-documented encoding, and record it explicitly as a
de-facto-standard model choice rather than silicon-documented behavior.**

- Above-4 GB memory is reported at CMOS `5Bh` (bits 7:0), `5Ch` (bits 15:8) and
  `5Dh` (bits 23:16) as a count of 64 KiB units.
- Wherever it is implemented, the constants and the doc comment must say that
  the source is the Bochs/QEMU convention as consumed by SeaBIOS, **not** a
  datasheet, and `docs/sources.md` must classify it the same way.
- This is an interoperability decision of the same kind as ADR-0005: the
  encoding is a fact two implementations must agree on, discoverable from
  documentation of the convention. It does not license reading QEMU's or
  Bochs's implementation of it.
- **Not implemented in round 2.** Nothing in the current tree configures more
  than 4 GB of guest RAM, so shipping the encoding now would add an untested
  path. The decision is what round 2 delivers; the code follows when a machine
  configuration can actually exercise it.

## Consequences

Easier: when large-memory guests arrive, the encoding is already settled and
does not need to be argued again under time pressure. The honesty rule is
preserved in a form that can survive contact with real firmware — the model
choice is labelled rather than dressed up as a specification.

Harder: this creates a second, weaker tier of source authority ("de-facto
standard") alongside vendor documentation. That tier must stay small and must
always be labelled at the point of use, or the distinction stops meaning
anything. Any future entry in it needs its own ADR.

Also worth stating plainly: until this is implemented, a guest asking this
machine about memory above 4 GB reads zero. That answer is wrong rather than
absent, and it is the reason the gap is recorded here instead of only in a
comment.
