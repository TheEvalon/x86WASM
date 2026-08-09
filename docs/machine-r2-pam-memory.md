# Machine memory notes — Milestone 2, Round 2 (PAM, shadowing, step clock, option ROMs)

Spec citations and model choices for the machine/firmware memory slices on
`slice/r2-machine-mem`. No emulator source was consulted; only the
specifications listed here and in `docs/sources.md`.

## 1. i440FX Programmable Attribute Map on `PhysMem`

Specs

- Intel 440FX PCIset — 82441FX PCI and Memory Controller (PMC) datasheet,
  PCI configuration registers `0x59`–`0x5F` (PAM0–PAM6, "Programmable Attribute
  Map"). Each register holds two 4-bit attribute fields; the low nibble
  attributes the lower-addressed region of the pair and the high nibble the
  higher-addressed one. Within a field, bit 0 is RE (read enable: reads are
  directed to DRAM) and bit 1 is WE (write enable: writes are directed to
  DRAM); bits 3:2 are reserved. When RE is clear a read is forwarded to PCI;
  when WE is clear a write is forwarded to PCI. Reset value is `0x00` for every
  register, so the whole legacy window reads from PCI (the BIOS ROM) and no
  write reaches DRAM.
- Region map: PAM0 bits 7:4 attribute `0xF0000`–`0xFFFFF` (BIOS area, 64 KiB);
  PAM0 bits 3:0 are reserved. PAM1–PAM6 attribute twelve 16 KiB regions in
  ascending order, `0xC0000`–`0xC3FFF` through `0xEC000`–`0xEFFFF`.
- PCI Local Bus Specification — Type 0 configuration header and the PCI
  behavior a cycle takes when the host bridge does not claim it (a write with
  no target is dropped, it is not an error signalled to the processor).

This source is **not yet listed in `docs/sources.md`**; the 440FX PMC datasheet
PAM section needs adding by the coordinator.

Supported

- Thirteen independently attributed regions (`PAM_REGIONS`), each with a
  `PamRead` (`Rom` / `ShadowRam`) and a `PamWrite` (`Ignored` / `ShadowRam`).
- Reset defaults: every region `Rom` / `Ignored`, i.e. PAM `0x00`.
  `Machine::reset` restores them (PCIRST# clears the registers) and leaves the
  shadow contents alone, the way DRAM keeps its bits.
- Register-level decode: `PhysMem::apply_pam_register(offset, value)` for
  offsets `0x59`–`0x5F` splits the byte into the two nibble fields, ignores
  PAM0's reserved low nibble, and masks the reserved bits 3:2.
  `PhysMem::pam_register_value` re-encodes the decoded view.
- Region-level API: `PhysMem::set_region_attributes(region, readable_from,
  writable_to)` plus `region_attributes`, `pam_region_index(addr)`,
  `pam_region_range(region)`, and `pam_region_for_register(offset, high_nibble)`.
- A write inside the PAM window never returns `MemError::RomWrite`: with WE set
  it lands in shadow DRAM, with WE clear it is dropped. Outside the window
  (notably the top-of-4 GiB BIOS map) a ROM write still returns `RomWrite`, so
  the existing lab-ROM diagnostic is unchanged.
- The A20 mask is applied before the PAM decode, so a masked `0x1F0000` access
  attributes as `0xF0000`.
- `PhysMem::is_mapped` reports a region reading from shadow DRAM as mapped, so
  the POST probe does not log shadowed firmware as unimplemented MMIO.

Model choices (not hardware-accurate)

- **Read fall-through.** With RE clear this model reads the ROM window covering
  the address and, when no ROM window covers it, falls through to the ordinary
  RAM / open-bus decode instead of forcing the cycle to PCI. A real 440FX would
  never return DRAM content for RE=0. Keeping the fall-through means machines
  built without an option ROM behave exactly as they did before this slice.
- **Shadow backing store.** Shadow reads and writes use main DRAM when the
  machine has RAM at that physical address. When it does not (small lab
  machines), a 256 KiB legacy buffer covering `0xC0000`–`0xFFFFF` is allocated
  on first shadow write and reads as zero before that. On real hardware the
  shadow *is* main DRAM; the auxiliary buffer exists so a 64 KiB test machine
  can still exercise shadowing.
- `pam_register_value` reconstructs the register from the decoded attributes,
  so reserved bits read back as zero. The byte-exact register file belongs to
  the PCI side.

Not supported

- The PMC's other DRAM-controller registers: DRB row boundaries, DRAMC, DRAMT,
  FDHC fixed-DRAM-hole control, MTT, SMRAM (`0x72`) and the SMM address space,
  memory-space gap registers, and error/ECC reporting.
- The PCI side of the register: nothing writes `0x59`–`0x5F` yet. The wiring
  from `devices::PciConfig` is a separate agent's slice; see "Host API for the
  PCI-side caller" below.
- No caching attributes (the datasheet's cacheability interaction), no write
  combining, and no distinction between a code fetch and a data read.

## Host API for the PCI-side PAM caller

The PCI configuration registers live in `devices::PciConfig`; the memory model
lives in `machine_pc::PhysMem`. `devices` cannot depend on `machine-pc`, so the
machine layer is where the two halves meet. After a PCI configuration write to
host-bridge `00:00.0` offsets `0x59`–`0x5F`, call:

```rust
// Machine level (preferred wiring point, e.g. from MachineBus after a
// PciConfig write that touched a PAM offset):
machine.apply_pam_register(offset /* 0x59..=0x5F */, value); // -> bool
machine.pam_register(offset);                                // -> Option<u8>

// PhysMem level (what the Machine helpers forward to):
mem.apply_pam_register(offset, value);                       // -> bool
mem.set_region_attributes(region, PamRead::ShadowRam, PamWrite::Ignored); // -> bool
mem.region_attributes(region);                               // -> Option<PamAttributes>
PhysMem::pam_region_for_register(offset, high_nibble);       // -> Option<usize>
PhysMem::pam_region_index(phys_addr);                        // -> Option<usize>
```

`apply_pam_register` returns `false` for an offset outside `0x59`–`0x5F` and
`set_region_attributes` returns `false` for a region index outside
`0..PAM_REGION_COUNT`. Region indices are in ascending address order:
`0` = `0xC0000`–`0xC3FFF` … `11` = `0xEC000`–`0xEFFFF`, and
`PAM_BIOS_REGION` (`12`) = `0xF0000`–`0xFFFFF`.

## 2. BIOS shadowing end to end

Specs

- Intel 440FX PMC PAM as above. The shadowing sequence is: set the region to
  read-from-ROM / write-to-DRAM, copy the region onto itself (reads resolve to
  the ROM, writes land in DRAM), then set read-from-DRAM / write-disabled so
  the copy is what executes and stray writes are dropped.
- Intel SDM Vol. 3 §9.1.4 — reset fetch at `0xFFFFFFF0` with
  `CS.base = 0xFFFF0000`; a far `JMP ptr16:16` to `F000:0000` moves execution to
  the below-1 MiB alias, which is the window PAM attributes.
- SeaBIOS memory map (`docs/sources.md`, Firmware) — the BIOS is mapped at the
  top of 4 GiB with the last up to 128 KiB aliased below 1 MiB.

Supported

- `crates/machine-pc/tests/bios_shadow.rs` runs the sequence with guest
  instructions (`REP MOVSB` twice over 32 KiB, entered through the reset
  vector's far jump), patches one byte of the shadow copy while writes still
  reach DRAM, locks the region, and then resumes so the next instruction can
  only produce its output if the **fetch** came from shadow DRAM.
- The same test checks that the top-of-4 GiB window still returns the original
  image after the low alias has been shadowed and re-attributed, so
  `firmware_interface::prepare_bios_rom`'s dual placement keeps working.
- A second test drives a guest `MOV ES:[DI], AL` into the BIOS area with PAM at
  its reset value and shows the machine continues instead of taking a memory
  fault — the behavior that would otherwise stop POST.
- A third test shadows the `0xE0000` region of a 256 KiB image and shows the
  neighbouring 16 KiB region and the high map are untouched.

Not supported

- Nothing arms PAM automatically. Until the PCI side forwards writes to
  `0x59`–`0x5F`, a guest that copies itself into the BIOS area still sees its
  writes dropped, which is what real hardware does before PAM is programmed.
- The copy is a plain byte copy: no cacheability, no write combining, and no
  modeling of the fetch-versus-data distinction during the transition.

## 3. Instruction-count step clock

Specs

- Intel 8254 Programmable Interval Timer datasheet — the counter is clocked by
  the external CLK input; the IBM PC/AT drives it at 1.193182 MHz
  (14.31818 MHz ÷ 12).
- Motorola MC146818A — the periodic interrupt rate comes from the Status A RS
  field (POST default `0110b` = 1024 Hz), and the update cycle runs once per
  second; Status C UF latches at update-ended.
- Intel 8259A — master IR0 carries PIT channel 0.

Model choice (explicitly not accurate timing)

- Device time is tied to **retired instructions**, not wall clock, so a run is
  reproducible. Each retired instruction charges `pit_clocks_per_step` PIT
  input clocks (default 1). Every `PIT_CLOCKS_PER_CMOS_PERIOD` (1165 =
  1_193_182 ÷ 1024) accumulated clocks runs one `tick_cmos` period, and every
  `PIT_CLOCKS_PER_SECOND` (1_193_182) accumulated clocks runs one
  `tick_cmos_second`. Remainders carry across steps.
- With the default ratio the guest sees a machine retiring 1.193182 million
  instructions per emulated second. That number is **not** derived from any
  processor's IPC and is not host real time; firmware that measures the CPU
  against the PIT will compute a nonsense frequency. The ratio exists so timer
  polling loops terminate deterministically, and it is configurable
  (`StepClock::with_pit_clocks_per_step`).
- The clock is **off by default**, so `Machine::step` behaves exactly as before
  and every existing test that ticks devices by hand is unaffected.
- `Machine::probe_post` arms `StepClock::enabled_default()` for the duration of
  a run when the host has not configured one, and restores the previous
  configuration afterwards. A host-configured clock is used as-is. The probe is
  a diagnostic, and a firmware `usleep` that never returns measures nothing.
- Only a **retired** instruction charges the clock: a step that fails does not,
  and the charge happens before a latched 8042 / port `0x92` system reset is
  serviced, so a reset never sees ticks applied to freshly reset devices.
- `Machine::reset` drops partial quanta and keeps the configuration, matching
  the way host-configured fw_cfg state survives a reset.

Not supported

- No wall-clock or host-monotonic source, no TSC / HPET / APIC timer, no
  per-instruction cost model (every instruction is one step), and no PIT gate
  or latency modeling beyond the existing device model.
- The RTC periodic quantum is derived from the nominal 1024 Hz POST default,
  not from the guest's current Status A RS field, so reprogramming RS does not
  change how fast periods accumulate.
- PIT channels 1 and 2 are advanced only as far as the existing `tick_pit`
  helper does (channel 0 plus channel 2); the channel-1 refresh counter still
  needs `tick_ch1`.

### POST probe opcode reporting

The stop-reason line named a two-byte opcode by its second byte, so the pinned
SeaBIOS stop printed `unsupported opcode 0x85` for what is really `0F 85`. The
decoder only reports the byte its tables missed, so `PostFailure` now
reconstructs the site from the captured window (`OpcodeSite::from_window`):
legacy prefixes per Intel SDM Vol. 2 §2.1.1 are skipped and reported
separately, and `0F` / `0F 38` / `0F 3A` escapes are included, giving
`unsupported opcode 0x0F 0x85` and `... (prefixes 66)` when prefixed. The
`PostFailureKind::UnsupportedOpcode(u8)` payload is unchanged — it is still the
decoder's byte — and the reconstruction is used only when its final opcode byte
agrees with that payload. REX and VEX/EVEX prefixes are not recognized (no
long mode here).

## 4. Option ROM mapping at `0xC0000`

Specs

- PCI Firmware Specification / BIOS Boot Specification, PC-compatible expansion
  ROM header: byte 0-1 signature `0x55 0xAA`, byte 2 the initialization size in
  512-byte blocks, byte 3 onwards the entry point; the byte-wise sum over the
  initialization size must be zero modulo 256.
- Legacy placement: the option-ROM region is `0xC0000`-`0xDFFFF`, scanned on
  2 KiB boundaries; the video BIOS is conventionally at `0xC0000`.
- Intel 440FX PMC PAM1-PAM4 (`0x5A`-`0x5D`) attribute that same region, so an
  option ROM can be shadowed exactly like the BIOS area.

This source is **not yet listed in `docs/sources.md`**; the PCI Firmware
Specification / BIOS Boot Specification expansion-ROM header entry needs adding
by the coordinator.

Supported

- `firmware_interface::prepare_option_rom(phys_base, data)` validates the
  signature, a non-zero size byte, that the declared size fits inside the
  supplied image, the checksum over the declared size, 2 KiB base alignment,
  and containment in `0xC0000`-`0xDFFFF`, returning a `RomImage` carrying
  exactly the declared bytes. `OptionRomError` names each rejection.
- `Machine::map_option_rom(phys_base, data)` adds that window alongside the
  BIOS windows; `Machine::map_vga_option_rom(data)` is the `0xC0000` case.
- `crates/machine-pc/tests/option_rom.rs` checks what a BIOS scan would see
  (signature, size byte, zero checksum over the mapped image, open bus at an
  empty 2 KiB slot on a 640 KiB machine), a guest reading the signature at
  `C000:0000`, PAM shadowing of the region, rejection of a bad checksum, and
  coexistence with the BIOS windows.

Not supported

- No PCI expansion-ROM BAR (`0x30`) and no ROM discovery through PCI: the host
  places the image explicitly.
- No PnP expansion header (offset `0x1A`), no runtime-size versus
  initialization-size distinction, no BEV/BCV boot entries, and no automatic
  packing of several ROMs into the region.
- `Machine::load_bios_rom` clears every ROM window, so option ROMs must be
  mapped after the BIOS image; there is no window registry that survives a
  BIOS reload.
- Nothing executes the ROM: there is no INT 19h / INT 10h dispatch, no entry
  call at `base+3`, and no real VGA BIOS image in this tree (a sibling slice
  owns that).
