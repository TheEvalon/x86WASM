# Writes this platform cannot store — Milestone 2, Round 4, slice 1

What an i440FX/PIIX PC does with a memory write that nothing accepts, and why
this model stopped raising `#GP` for one third of that set.

## The measured defect

Round 3 left SeaBIOS POST spinning. From trace event 315 onward the run
alternated 1:1 between `mem-fault wr addr=0xFFFF6E06` and `OUT 0x20, 0x20`
(master PIC EOI), **37,060 times**, consuming the rest of a 2,000,000-step
budget. `0xFFFF6E06` is `CS.base = 0xFFFF0000` plus `0x6E06` — a store into the
top-of-4 GiB BIOS alias while firmware still executed from it.

The path was: `PhysMem::write_u8` returned `MemError::RomWrite`, `MachineBus`
turned that into `ExecError::MemoryFault`, the interpreter classified it as
`#GP` (vector 13), SeaBIOS's handler EOI'd and returned, and the same store
retried forever.

The fault was the bug.

## What the specifications say

### 1. The processor has no exception for this

Intel SDM Vol. 3A §6.15 enumerates the sources of `#GP` for a data write:
segment limit, segment type, null selector, non-canonical address. Paging adds
`#PF` (Vol. 3A §4.7). There is no entry for "the platform declined the store".
A bus response can only reach the processor through the machine-check
architecture (Vol. 3B Chapter 16), which requires `CR4.MCE` and enabled banks —
neither of which exists at reset, and neither of which this model implements.
So the correct emulation of *any* write the platform cannot store is: complete
the instruction, store nothing.

### 2. PCI drops an unclaimed write

PCI Local Bus Specification Revision 3.0 §3.2.2.3.4 "Master-Abort Termination":
when no agent asserts DEVSEL# within the decode window the master terminates
the transaction with Master-Abort. For a write, the data is discarded. The
initiator records Received Master Abort in its Status register; signalling the
condition further (SERR#) is separately enabled and off at reset. Nothing about
this reaches the processor as an exception.

### 3. The 440FX forwards a write-disabled region to PCI

Intel 440FX PCIset — 82441FX (PMC) datasheet §3.2.18 (PAM0–PAM6, Table 2): WE
set directs CPU writes of the segment to main memory; WE clear forwards the
cycle to PCI. Combined with §3.2.2.3.4 that is a dropped write, which is what
this model already did inside `0xC0000`–`0xFFFFF`.

### 4. A ROM never claims a write cycle

Intel 82371SB (PIIX3) datasheet, X-Bus Chip Select register (XBCS, config
offset `4Eh`): BIOSCS# is generated for the `0E0000h`–`0FFFFFh` region (bit 7,
Lower BIOS Enable), for `FFF80000h`–`FFFFFFFFh` / the top-of-4 GiB alias
(bit 6, Extended BIOS Enable), and is asserted for **write** cycles only when
BIOS write protection is disabled (bit 2). With protection in force the ROM
does not claim the cycle at all, so it terminates exactly like case 2. With
protection lifted the cycle reaches a mask ROM or an unsequenced flash part,
which likewise stores nothing. Either way: no error to the processor.

**This source is not yet listed in `docs/sources.md`** — the PIIX3 XBCS entry
needs adding by the integrator.

### 5. The top-of-4 GiB alias is outside PAM, deliberately

Intel SDM Vol. 3A §9.1.4: the first instruction after reset is fetched from
`0xFFFFFFF0` with `CS.base = 0xFFFF0000`. PAM attributes only the thirteen
segments in `0xC0000`–`0xFFFFF` (PMC Table 3), so the high alias has no PAM
attribute to consult and cannot be shadowed. On real hardware its behavior is
governed by PIIX3 XBCS rather than by the PMC, and that behavior — per §4 — is
to drop writes. It therefore gets the *same* answer as the PAM window, by a
different mechanism, which is why the three cases can be unified without
pretending the mechanisms are the same.

## The decision

`PhysMem::write_u8` never fails. Every write that cannot be stored is dropped
and the instruction completes:

| Case | Mechanism | Before | After |
|---|---|---|---|
| Mapped ROM outside PAM (`0xFFFF0000` alias, lab ROMs) | PIIX3 BIOSCS# not asserted for writes | `Err(RomWrite)` → `#GP` | dropped |
| PAM region, WE clear | PMC forwards to PCI, Master-Abort | dropped | dropped |
| Unclaimed physical space | PCI Master-Abort | dropped | dropped |

`PhysMem::write_u8_classified` returns a [`WriteDisposition`] naming which of
the four outcomes occurred (`Accepted`, `DroppedRom`,
`DroppedPamWriteDisabled`, `DroppedUnclaimed`). This is a **host diagnostic**;
the guest cannot distinguish the three dropped cases, which is the point of the
change. `MemError::RomWrite` is retained as a name for the ROM case, but
`write_u8` no longer returns it.

`MachineBus::write_u8` records a `PostTraceEvent::RomWriteDropped` for the
`DroppedRom` case only. That is deliberate asymmetry in the *log*, not in the
architecture: unclaimed writes already appear in the probe's aggregated
`unmapped-mmio` page log, and PAM-disabled writes are the ordinary state of a
locked BIOS during POST and would drown the ring. A write to mapped ROM had a
visible trace event before this slice and keeps one after it.

## Tests changed, and why

Four existing tests asserted the fault this slice removes. Each was pinning the
defect, not a specification:

- `mem::tests::rom_overrides_ram` — asserted `Err(MemError::RomWrite)` for a
  write over a lab ROM. Now asserts the write completes and the ROM keeps its
  byte.
- `mem::tests::dual_rom_windows_high_and_low_alias` — asserted the high map
  "still reports the diagnostic". Now asserts both maps behave the same way to
  the guest.
- `mem::tests::pam_does_not_touch_high_rom_window` — same, and now uses
  `write_u8_classified` to keep the *diagnostic* distinction it was really
  testing (re-attributing the low alias does not make the high map writable).
- `machine_pc::tests::load_bios_rom_maps_high_and_f0000_alias` — same.
- `tests/post_trace.rs::a_memory_fault_is_recorded_next_to_the_accesses_that_led_to_it`
  — asserted a `MemoryFault` event at `0xFFFF_0000`. Replaced by
  `a_write_into_rom_is_dropped_recorded_and_does_not_stop_the_run`, which
  additionally checks that the guest **keeps running** to its `HLT` and reads
  the original ROM byte back. The new assertions are stronger: the old test
  could not have distinguished "faulted" from "faulted and stopped POST".

New: `mem::tests::every_dropped_write_case_completes_and_they_are_distinguishable`
drives all three cases plus an accepted write in one place, which is the
property this slice is actually about.

## Measured effect on SeaBIOS POST

Before (round 3 tip, `--post-trace`): 74,435 platform events, ending in 37,060
`mem-fault` / `OUT 0x20,0x20` pairs.

After: **319 platform events total**, of which the `0xFFFF6E06` store appears
exactly once, as four `rom-write … dropped` byte events (one 32-bit store), and
firmware continues. The `--post-probe` output is unchanged only because a
second, unrelated blocker consumes the budget — see
`docs/machine-r4-fseg-sweep.md`.

## Not supported

- No machine-check architecture, so there is no way for a host to ask for the
  strict behavior a real chipset could be configured into (SERR# on
  Master-Abort → NMI). If that is ever wanted it belongs on the PCI Status
  register and the NMI path, not on `PhysMem`.
- PIIX3 XBCS is not modeled as a register: BIOS write protection is implicitly
  always in force. Nothing in this tree writes `4Eh`, and a guest cannot make
  the BIOS ROM writable through it.
- The PMC's Received Master Abort status bit is not set by a dropped memory
  write; only the configuration-cycle path models master-abort status today.
