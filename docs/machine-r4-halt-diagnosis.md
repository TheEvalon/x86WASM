# Why SeaBIOS halts at 150,360 — Milestone 2, Round 4, slice 4

Two things: a machine-scope defect this slice fixes (a wake-able `HLT` was
treated as a stop), and the actual cause of the 150,360-step halt, which is a
`devices::PciConfig` defect and is handed off rather than fixed here.

## The measurement

With the `x86-interpreter` `moffs` address-size fix applied (see
`docs/machine-r4-fseg-sweep.md`; the same one-line fix a sibling landed on
`slice/r4-paging-wire`):

```text
post-probe: steps=150360 stop=halted
  stop-pc        cs:ip=0008:B235 cs.d=1 eip=0x000EB235 linear_pc=0x00000000000EB235 bytes=[F4 EB FD 8D 44 24 6C E8]
  spin           sampled=4096 window=4096 distinct=337 cycle=none
  spin-pc        count=169 cs:ip=0008:6E26 ... linear_pc=0x00000000000E6E26
  spin-pc        count=169 cs:ip=0008:6E28 ... linear_pc=0x00000000000E6E28
  spin-pc        count=169 cs:ip=0008:6E3B ... linear_pc=0x00000000000E6E3B
  spin-pc        count=168 cs:ip=0008:6E2A ... linear_pc=0x00000000000E6E2A
  post-codes=[] last=none
  com1="" debug=""
```

`bytes=[F4 EB FD …]` is `HLT` followed by `JMP $-1`: a hang loop, not an idle.
Backing up one more byte in the image gives `FA F4 EB FD` — `CLI; HLT; JMP $-1`,
the shape of a firmware `panic()` compiled with debug output off.

## Why the halt is silent

This pinned SeaBIOS `rel-1.16.3` build has diagnostic output compiled out.
Across a full run it touches only ports `0x0D`, `0x20`, `0x21`, `0x70`, `0x71`,
`0x92`, `0xA0`, `0xA1`, `0xD4`, `0xD6`, `0xDA`, `0x510`, `0x518` — **never**
`0x402`, `0x3F8`, or `0x80`. So `post-codes=[]` and `com1="" debug=""` are
properties of the firmware build, not gaps in this machine: no checkpoint or
console output is coming from this image no matter what we fix. Diagnosis has to
come from the probe, which is what slice 3 is for.

## The panic condition, traced

The last instructions before the halt:

```text
000EB22A  E8 5E A8 FF FF    call 000E5A8D
000EB22F  85 C0             test %eax,%eax
000EB231  74 04             je   000EB237
000EB233  FA F4 EB FD       cli ; hlt ; jmp $-1
```

`EAX = 0xFFFFFFFF` on return, so the guard fails. Its tail computes a 64-bit
base and range-checks it:

```text
000E5B05  F7 D8 / 83 D2 00 / F7 DA   negate the 64-bit alignment -> mask
000E5B0C  21 C6 / 21 D7              ALIGN_DOWN(base, align)
000E5B10  89 75 00 / 89 7D 04        store the aligned base
000E5B16  3B 35 F8 0A 0F 00          cmp  esi, [0x000F0AF8]      \ 64-bit
000E5B1C  89 F8 / 1B 05 FC 0A 0F 00  sbb  eax, [0x000F0AFC]      / base < floor?
000E5B24  0F 92 C1                   setb cl
000E5B27  BB 00 00 C0 FE             mov  ebx, 0xFEC00000        ; I/O APIC base
000E5B2E  39 F3 / 19 F8              64-bit compare 0xFEC00000 < base
000E5B32  0F 92 C0 / 09 C8           setb al ; al |= cl
000E5B3A  F7 D8                      neg  eax                    ; -> 0 or -1
```

At the check the alignment register pair held `0x00000000_FFFF0020` and the
aligned base came out as `0xFFFFFFFE_0000EFC0`, which is above `0xFEC00000`, so
the guard returns `-1` and the firmware hangs. This is PCI resource assignment
failing to place the device windows below the I/O APIC.

`0xFFFF0020` is not a power of two, so the alignment itself is already garbage.
It is exactly what a BAR size computation produces from a read-back of
`0x0000FFE0`: `~(0x0000FFE0 & ~0x3) + 1 == 0xFFFF0020`.

## The defect: BAR sizing read-backs

PCI Local Bus Specification Revision 3.0 §6.2.5.1 "Base Addresses": software
sizes a BAR by writing all ones and reading it back; the device returns zeros in
the bits it does not implement, and **unimplemented base address registers are
hardwired to zero**. §6.2.5.2 says the same for the Expansion ROM base address
register.

Every BAR in `devices::PciConfig` violates this. From one traced run:

| Target | Register | Written | Read back | Should read back |
|---|---|---|---|---|
| `00:01.2` UHCI | `0x20` (32-byte I/O BAR) | `FFFFFFFF` | `0000FFE1` | `FFFFFFE1` |
| `00:01.1` IDE | `0x20` (BMIBA, 16-byte I/O) | `FFFFFFFF` | `0000FFF1` | `FFFFFFF1` |
| `00:00.0`, `00:01.0`, `00:01.3` | `0x10`–`0x24` (unimplemented) | `FFFFFFFF` | `FFFFFFFF` | `00000000` |
| every function | `0x30` (no expansion ROM) | `FFFFFFFF` | `FFFFF800` | `00000000` |

Two distinct mistakes:

1. **Implemented BARs mask off the address bits instead of the size bits.** A
   32-byte I/O BAR must keep bits 31:5 writable and hardwire 4:1 to zero and
   bit 0 to one. `PciConfig` instead stores the value ANDed with the *size*
   mask, so the high address bits read back as zero and firmware computes a
   ~4 GiB region from a 32-byte aperture. This is the direct source of
   `0xFFFF0020`.
2. **Unimplemented BARs are plain read/write dwords.** They must be hardwired
   to zero so firmware skips them. Reading back `FFFFFFFF` advertises six extra
   apertures per function on five functions, and `FFFFF800` advertises a 2 KiB
   option ROM on devices that have none.

`crates/devices/**` is not this slice's to change, so this is a hand-off, not a
fix. The PCI owner needs, per §6.2.5.1:

- a per-register writable mask (`~(size - 1)` for the address bits, with the
  type bits hardwired: bit 0, and bits 2:1 plus bit 3 for memory BARs),
- zero for every register with no aperture behind it, including `0x30`,
- a device-level test that writes all ones, reads back, and asserts the
  computed size equals the aperture the model actually decodes.

The round-3 note in `plan.md` predicted exactly this ("BAR sizing via an
all-ones write followed by a read-back is expected to be the first
misbehavior"). It is now measured rather than predicted.

## What this slice does fix: a wake-able `HLT` is an idle, not a stop

`Machine::probe_post` ended a run the instant `CPU.halted` became true, before
the platform had a chance to deliver anything. Every firmware timer idle —
`usleep`, `yield`, wait-for-device — therefore looked identical to a permanent
hang, and stopped the probe at the first `HLT`.

Specs:

- **Intel SDM Vol. 2 `HLT`**: the processor remains in the HALT state until an
  enabled interrupt (including NMI and SMI), a debug exception, BINIT#/INIT# or
  RESET# resumes execution. **Vol. 3A §6.8.1**: a maskable interrupt resumes it
  only with `IF = 1`.
- **Intel 8254**: the counters are clocked by the CLK input regardless of what
  the processor is doing, so the timer keeps counting through a halt.
- **Intel 8259A**: an unmasked IR with its IRR bit set drives INTR, which is
  what the halted processor waits on.

A halt with `IF = 1` is now an idle quantum: the probe keeps running, which
advances the instruction-count time source, which lets PIT channel 0 reach
terminal count and raise IRQ0. A halt with `IF = 0` ends the probe exactly as
before, with the same `stop=halted` text. `PostReport::idle_steps` counts the
wait; idle quanta retire no instruction so they are not in `steps`, they are not
sampled into the spin window (a long wait would otherwise bury the code that led
into it), and they draw on the same budget so a wait nobody can end is bounded
rather than hanging the host.

`crates/machine-pc/tests/halt_idle.rs` drives the whole path from the guest:
program the IVT, initialise the 8259A (ICW1–ICW4 then OCW1), program 8254
channel 0 in mode 0, `STI`, `HLT`, and check the handler ran and the guest
resumed. Two companion tests pin `CLI; HLT` stopping on the spot with
`idle_steps = 0`, and an idle with every IR masked being bounded by the budget.

SeaBIOS is unaffected today — its halt is `CLI; HLT`, i.e. `IF = 0` — which is
exactly why the change is safe to land now and why it was worth landing before
firmware reaches its first real idle.

## Also in this slice: the stop site carries its bytes

`PostReport::stop_bytes` gives the eight-byte instruction window at the stop, so
`stop=halted` reads as `bytes=[F4 EB FD …]` rather than an address alone. For a
halt the window backs up one byte onto the `HLT` itself, because `HLT` is the
single-byte opcode `F4` (Intel SDM Vol. 2) and `stop_site` is the resume point.

## Not supported

- **No NMI wake.** `CLI; HLT` could be ended by an NMI on real hardware; this
  machine has no autonomous NMI source, so `IF = 0` is treated as terminal.
  That stops being correct the moment a watchdog or parity source exists.
- **No SMI, INIT, or debug-exception wake.**
- **Idle quanta are instruction quanta**, with no defined relationship to real
  time under the instruction-count step clock.
- **No symbolisation.** The report names addresses and bytes; mapping them to
  firmware functions is still manual.
