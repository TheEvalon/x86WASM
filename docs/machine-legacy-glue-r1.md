# Machine glue notes — Milestone 2, Round 1 (legacy support devices + firmware wiring)

Spec citations for the machine/firmware wiring slices on `slice/r1-machine-glue`.
Each section records what the machine now supports and what it explicitly does
**not**. No emulator source was consulted; only the specifications listed here
and in `docs/sources.md`.

## 1. COM1/COM2 THRE → PIC IRQ4 / IRQ3

Specs

- National Semiconductor NS16550A UART — Interrupt Enable Register bit 1
  (ETBEI, transmitter-holding-register-empty interrupt enable) and Interrupt
  Identification Register interrupt ID `010b` (THRE), cleared by reading IIR or
  by writing THR.
- IBM PC/AT Technical Reference — ISA interrupt assignment: serial port 1
  (`0x3F8`–`0x3FF`) is IRQ4 on 8259A master IR4; serial port 2
  (`0x2F8`–`0x2FF`) is IRQ3 on master IR3.
- Intel 8259A datasheet — edge-triggered IR latching (ICW1.LTIM = 0, PIIX ELCR
  bit clear): IRR latches on the low→high transition and is cleared at INTA, so
  a held-high pin does not redeliver after EOI.
- Intel SDM Vol. 3 §6.8.1 — maskable interrupt delivery; vector = ICW2 base | IR.

Supported

- `MachineBus::poll_external_irq` drives master IR4 from `com1.irq_line()` and
  IR3 from `com2.irq_line()` (level follow) before acknowledge, alongside the
  existing PIT0/keyboard/FDC/CMOS/aux/IDE sources.
- `Machine::sync_com1_irq4` / `Machine::sync_com2_irq3` expose the same host-side
  level follow used by the other device sync helpers.
- A THRE interrupt with `IF=1` vectors through real-mode IVT entry `0x0C`
  (COM1) or `0x0B` (COM2) with the AT ICW2 base of `0x08`.

Not supported

- Received-data-available (IER bit 0 / IIR `100b`): the 16550 subset has no
  receive path, so `RBR` reads 0 and the line is never raised by RX.
- Receiver line status (IER bit 2 / IIR `110b`), modem status (IER bit 3 /
  IIR `000b`), FIFO control and FIFO-timeout interrupts, and MCR OUT2 gating of
  the ISA interrupt driver.
- IRQ4/IRQ3 sharing between COM1/COM3 and COM2/COM4 (no COM3/COM4 exist).

## 2. fw_cfg DMA interface at `0x514`

Specs

- QEMU Firmware Configuration (fw_cfg) Device,
  <https://www.qemu.org/docs/master/specs/fw_cfg.html> — "Register Locations"
  (x86: selector `0x510`, data `0x511`, DMA address `0x514`), "Revision /
  feature bitmap" (bit 0 traditional interface, bit 1 DMA interface),
  "Signature" (reading the DMA address register returns `0x51454d5520434647`,
  `QEMU CFG` big-endian), "Data Register" (reads past the end of an item return
  `0x00`), and "Guest-side DMA Interface" (64-bit big-endian address register,
  write to the least significant half at offset 4 triggers, `FWCfgDmaAccess`
  layout, control bits 0 error / 1 read / 2 skip / 3 select / 4 write, control
  writeback with all bits clear on success).

Only the specification document was used. No QEMU source was read or copied.

Supported

- `0x514`–`0x51B` decode as the 64-bit big-endian DMA address register. Writing
  the high half at `0x514` latches; writing the low half at `0x518` triggers.
  Reads return the `QEMU CFG` signature at byte granularity.
- Select (control bit 3) applies the upper 16 bits as the selector and resets
  the data offset, exactly like a selector-register write.
- Read (control bit 1) copies `length` bytes of the current selector/offset into
  guest RAM at `address`, zero-filling past the end of the item.
- Skip (control bit 2) advances the offset without touching guest memory.
- The control word is written back to the `FWCfgDmaAccess` structure: all bits
  clear on success, bit 0 set on error. The address register reads back as 0
  after an operation.
- `FwCfg::service_dma` takes guest-memory callbacks, so the device crate stays
  host-free; `MachineBus` supplies `PhysMem` accessors after every fw_cfg port
  write, which means the A20 gate applies to fw_cfg DMA as it does to the CPU.
- ID bit 1 is set only while the DMA register is live
  (`FwCfg::set_dma_enabled(false)` clears the bit and returns `0x514` to open
  bus), keeping the feature bitmap truthful.

Not supported

- Write direction (control bit 4). Item writeability is not modeled, so a write
  request is rejected with the error bit instead of mutating an item. Truncated
  writes, read-only rejection, and resize prevention therefore do not apply.
- Byte or word accesses to the address register, and 64-bit single writes (the
  port bus is 32 bits wide); only the two 32-bit halves are accepted.
- Asynchronous operation: transfers complete before the triggering `OUT`
  retires, so the "transfer still in progress" control state never appears.
- MMIO register layout (base+0 data / base+8 selector / base+16 DMA) and the
  full firmware blob set (ACPI/SMBIOS tables, boot order, kernel/initrd).

## 3. POST first-contact harness

Specs

- Intel SDM Vol. 3 §9.1.4 — processor state after reset: first instruction is
  fetched from `0xFFFFFFF0` with `CS.selector = 0xF000` and
  `CS.base = 0xFFFF0000`.
- Intel SDM Vol. 3 §3.4.2 — real-mode linear address is the cached segment base
  plus the 16-bit offset; instruction fetch wraps `IP` at 16 bits.
- Intel SDM Vol. 2 §2.1.1 — instructions are at most 15 bytes, so an eight-byte
  window is a prefix of the faulting instruction, not the whole of it.
- `docs/sources.md` (Firmware) — SeaBIOS is mapped at the top of 4 GiB with the
  last 128 KiB aliased below 1 MiB.

Supported

- `Machine::probe_post(max_steps)` single-steps mapped firmware and returns a
  `PostReport`: retired step count, a classified `PostStopReason`
  (`Halted`, `StepBudgetExhausted`, or `Failure`), the unclaimed I/O ports
  touched, the unmapped physical pages touched, and the COM1 / `0x402` output.
- A `Failure` records the kind (unsupported opcode, truncated instruction,
  instruction too long, unsupported encoding, memory fault, architectural fault
  with vector and error code, or protected-mode delivery failure) together with
  `CS:IP`, RIP, the linear PC, and the eight-byte wrapping opcode window. A
  two-byte `0F` escape is visible in that window even though the decode error
  names only the second byte.
- Diagnostic logging is armed only while the probe runs, so ordinary execution
  is unchanged. Both logs are bounded (64 distinct port sites, 32 distinct
  4 KiB pages) with explicit overflow flags.
- `emulator-cli --post-probe` runs the same harness over `--rom` / `--bios` and
  prints the report.
- `machine_pc::seabios_image_path` resolves `firmware/seabios/bios.bin` (or
  `X86WASM_SEABIOS_BIOS`) and returns `None` so tests skip gracefully when the
  git-ignored image has not been built.

Not supported

- This is a diagnostic only. It does not make POST succeed, does not resume past
  a failure, and does not enumerate blockers beyond the first CPU-level one.
- No time source is advanced during a probe, so firmware that polls the PIT or
  RTC will exhaust the step budget rather than progress.
- Unmapped physical accesses are folded to 4 KiB pages and still behave as open
  bus (reads `0xFF`, writes dropped); they are recorded, not faulted.

### What the harness found against real SeaBIOS

Running the probe over the pinned SeaBIOS `rel-1.16.3` `bios.bin` stops after
two retired instructions:

```text
post-probe: steps=2 stop=unsupported opcode 0x85 cs:ip=F000:E062
  rip=0x000000000000E062 linear_pc=0x00000000000FE062
  opcode_bytes=[0F 85 8E F9 31 D2 8E D2]
```

The reset vector far-jumps to `F000:E05B`, which executes
`CMP DWORD PTR CS:[0x9228], 0` and then reaches a two-byte `0F 85` near `JNZ`.
Near `Jcc rel16/rel32` (`0F 80`–`0F 8F`) is not in the decode tables, so SeaBIOS
cannot get past its first branch. That gap lives in `x86-decode` /
`x86-interpreter`, outside this slice's ownership; it is reported as the top
blocker for the next round rather than fixed here.

## 4. POST checkpoint port `0x80`

Specs

- IBM PC/AT Technical Reference — the system board decodes `0x80` as the
  manufacturing diagnostic port. POST writes a checkpoint code before each test
  phase so a diagnostic card displays the code of the failing step. The board
  drives no read data for the port.
- OSDev Wiki "I/O Ports" — `0x80` is also the conventional I/O-delay target, so
  writes can far outnumber real checkpoints.
- Observed in the pinned SeaBIOS image: one `OUT 0x80, AL` site at `0xF35F6`.

Supported

- `PostCodePort` claims port `0x80` from the open-bus fallback and latches every
  write as a checkpoint code, exposing `last_code`, an ordered `history` bounded
  at 256 entries with an explicit overflow flag, and a total `write_count` that
  keeps counting past the bound (so I/O-delay traffic is visible without
  unbounded growth).
- Reads still return ISA open bus (`0xFF`), matching a board that drives no data.
- A wider-than-byte write latches the low byte only.
- `Machine::reset` clears the latch and history; `Machine::probe_post` clears it
  at the start of a run and reports `post_codes` / `last_post_code` /
  `post_code_overflow` in the `PostReport`.

Not supported

- No display or POST-card model, no chipset-specific extended POST ports
  (`0x84`, `0x300`, `0x680`), and no distinction between a genuine checkpoint
  write and an I/O-delay write beyond the separate write count.
- `0x80` is not a DMA page register in this machine (the 8237A page decode
  already excludes it), and this slice does not change that.
