# CMOS checksum-range durability

Which CMOS indices survive `CmosRtc::reset`, and why the answer has to be "at
least the whole checksum range".

## Authority

- IBM PC/AT Technical Reference — CMOS RAM is powered by the system battery, so
  the configuration POST writes survives a CPU or device reset.
- Ralf Brown's Interrupt List, CMOS `2Eh`/`2Fh` "Standard CMOS Checksum,
  High/Low Byte": "2Eh and 2Fh are as defined by the original IBM PC/AT
  specification and represent a byte-wise additive sum of the values in
  locations 10h-2Dh only, 00h-0Fh and 30h-33h are not included."
- RBIL CMOS `0Eh` (diagnostic status byte, Table C0005) and `0Fh` (shutdown
  status / reset code).
- RBIL CMOS `30h`/`31h` ("EXTENDED MEMORY IN KB") and `34h`/`35h` ("EXTENDED
  MEMORY >16M").

## The defect this fixes

Reset preserved thirteen scattered indices: `0Eh`, `0Fh`, `14h`, `15h`–`18h`,
`2Eh`, `2Fh`, `30h`, `31h`, `34h`, `35h`. Eleven of those are inside or adjacent
to the checksum range, but the range itself has 30 bytes — `10h`, `12h`,
`19h`–`2Dh` and the rest were cleared while `2Eh`/`2Fh` were kept.

That combination is incoherent. A checksum is a statement about the bytes it
covers; keeping the statement while erasing part of what it describes makes the
stored value silently wrong. Anything that programmed a floppy type or a disk
geometry got a checksum that validated before a reset and failed after one, with
no event to attribute it to. That is exactly the failure mode POST's diagnostic
byte `0Eh` bit 6 ("incorrect checksum") exists to report, arriving for a reason
that never happens on real hardware.

## The model

`CmosRtc::is_battery_backed(index)` is the single answer, and
`apply_reset_defaults` is its only consumer.

| Range | Reset behavior | Why |
|---|---|---|
| `00h`–`09h` | cleared | MC146818 time/calendar; this model has no host wall clock to carry forward |
| `0Ah`–`0Dh` | power-on defaults | Status A–D are device state, not configuration |
| `0Eh`–`2Fh` | **preserved** | Diagnostic byte, shutdown code, the whole `10h`–`2Dh` checksum range, and the `2Eh`/`2Fh` checksum bytes |
| `30h`/`31h`, `34h`/`35h` | **preserved** | Memory-size bytes POST reads to build its map |
| `32h`/`33h`, `36h`–`7Fh` | cleared | Century and vendor-specific area; nothing in this machine writes them |

The preserved block is contiguous by construction, and `const` assertions in
`cmos.rs` keep it containing `CMOS_CHECKSUM_FIRST`–`CMOS_CHECKSUM_LAST` and the
shutdown byte, so the two cannot drift apart in a later edit.

## Staleness is still detectable

Battery backing removes *reset* as a cause of a stale checksum. It does not stop
a guest from writing a covered byte and leaving the sum wrong, which is a real
thing firmware and setup utilities do between the write and the recompute.

`CmosRtc::standard_checksum_valid()` reports the mismatch, and the device still
takes no action on it — it never repairs the checksum, never sets `0Eh` bit 6,
and never refuses a read. Evaluating the checksum is POST's job, exactly as
before this slice. `cmos_battery_backed_range.rs` asserts that a mismatch
created before a reset is still visible as a mismatch after one.

## Deviations from hardware (explicit)

- Real battery-backed CMOS preserves all 128 bytes, including the clock. This
  model returns `00h`–`0Dh` to its power-on state, so guest-visible time does
  not advance across a reset and Status A–D come back at their documented
  defaults. That is a model choice driven by having no host time source, not
  MC146818 behavior.
- `32h`/`33h` and `36h`–`7Fh` are cleared rather than preserved. Nothing in this
  machine writes them today; when something does, they belong in the preserved
  set and the const assertions should be extended with it.
- The IBM PS/2 line does not use the `10h`–`2Dh` checksum definition at all
  (RBIL: "the range 19h-31h being undefined"). This model implements the
  AT/AMI/Compaq definition RBIL calls standard, which is what SeaBIOS-class
  firmware expects.
