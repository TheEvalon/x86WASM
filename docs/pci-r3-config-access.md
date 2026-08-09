# PCI configuration Mechanism #1 access widths

What `devices::PciConfig` does with every combination of port, width, and
alignment at `0xCF8`–`0xCFF`, and why.

## Authority

PCI Local Bus Specification Revision 3.0, §3.2.2.3.2 "Configuration Mechanism
#1" (Figure 3-2 "Layout of CONFIG_ADDRESS Register", Figure 3-3 "Host Bridge
Translation for Type 0 Configuration Transactions Address Phase", footnote 15)
and §3.2.2.3.4 "Selection of a Device's Configuration Space". Intel SDM Vol. 2
`IN`/`OUT` for the widths an x86 I/O access can carry.

The four sentences the implementation turns on:

1. "Bit 31 is an enable flag for determining when accesses to CONFIG_DATA are to
   be translated to configuration transactions on the PCI bus. Bits 30 to 24 are
   reserved, read-only, and must return 0's when read ... Bits 1 and 0 are
   read-only and must return 0's when read."
2. "Anytime a host bridge sees a full DWORD I/O write from the host to
   CONFIG_ADDRESS, the bridge must latch the data into its CONFIG_ADDRESS
   register. On full DWORD I/O reads to CONFIG_ADDRESS, the bridge must return
   the data in CONFIG_ADDRESS. Any other types of accesses to this address
   (non-DWORD) have no effect on CONFIG_ADDRESS and are executed as normal I/O
   transactions on the PCI bus."
3. "When a host bridge sees an I/O access that falls inside the DWORD beginning
   at CONFIG_DATA address, it checks the Enable bit and the Bus Number in the
   CONFIG_ADDRESS register."
4. Footnote 15: "the bridge must complete the processor access normally,
   dropping the data on writes and returning all ones on reads."

## CONFIG_ADDRESS — `0xCF8`–`0xCFB`

| Access | Behavior |
|---|---|
| 32-bit at `0xCF8` | Latches `value & 0x80FF_FFFC`; reads return the latch |
| 32-bit at `0xCF9`/`0xCFA`/`0xCFB` | Not CONFIG_ADDRESS. Ordinary I/O: read all ones, write dropped |
| 8/16-bit anywhere in `0xCF8`–`0xCFB` | Ordinary I/O: read all ones, write dropped |

Reserved bits 30:24 and 1:0 are never stored, so a read-back is byte-identical
to what `PciConfig::make_address` would have produced. Register selection is
bits 7:2 only: this is the 256-byte legacy configuration space, not the
extended (`0xCF8` bits 27:24 / MMCONFIG) space, which is unimplemented.

### Model choice: byte-lane compatibility policy

`PciConfig::set_config_address_byte_lane_compat(true)` makes 8- and 16-bit
accesses to `0xCF8`–`0xCFB` assemble the latch byte-lane-wise. **This is not
hardware behavior** — rule 2 above forbids it, and the Intel 440FX PMC
documents CONFADD the same way.

It exists for exactly one reason: the interpreter's primary opcode map has no
`EF` (`OUT DX, eAX`) form yet, so guest code in a machine-level test cannot
issue a 32-bit `OUT` at all. `crates/machine-pc/tests/pam_bus_seam.rs` arms the
policy, drives the BIOS-shadowing sequence with four byte stores, and says so.
The policy is off by default, survives `PciConfig::reset` (host configuration,
not guest state), and should be deleted once `EF` decodes.

## CONFIG_DATA — `0xCFC`–`0xCFF`

An access is translated into a configuration cycle only when **all** of:

- the width is 1, 2, or 4 bytes, and
- the access lies entirely inside the dword `0xCFC`–`0xCFF`, and
- CONFIG_ADDRESS bit 31 is set, and
- the latched bus/device/function selects a function this machine implements
  (bus 0; `00:00.0` host bridge, `00:01.0`–`00:01.3` PIIX).

Otherwise it is an ordinary I/O transaction that nothing claims: reads return
all ones, writes are dropped.

The lane rule is the one that was wrong before this slice. A configuration
transaction addresses one dword and carries four byte enables "directly copied
from the processor bus", so an access that runs past `0xCFF` cannot be
expressed as a single cycle. The previous code let a 16-bit read at `0xCFF`, or
a 32-bit access at `0xCFD`–`0xCFF`, fold into the *following* configuration
register — a 16-bit read of register `0x03` returned the low byte of the
Command register at `0x04`, and a 32-bit write at `0xCFE` could silently
overwrite it. Firmware that walks a header with mixed widths would have
corrupted neighbouring registers.

Legal byte-enable patterns, therefore:

| Port | 8-bit | 16-bit | 32-bit |
|---|---|---|---|
| `0xCFC` | byte 0 | bytes 0-1 | bytes 0-3 |
| `0xCFD` | byte 1 | bytes 1-2 | — |
| `0xCFE` | byte 2 | bytes 2-3 | — |
| `0xCFF` | byte 3 | — | — |

The unaligned 16-bit forms at `0xCFD` are legal: the byte enables `0110` are
representable, and the spec constrains the cycle, not the alignment.

## Absent targets

A read of any width from an unimplemented device number, function number, or a
non-zero bus (which would be a Type 1 cycle with no bridge behind this host
bridge) returns all ones, and writes are dropped — footnote 15 and §3.2.2.3.4
Master-Abort. This is what makes SeaBIOS's enumeration terminate: `00:01.4`
through `00:01.7` and every device number other than 0 and 1 report vendor ID
`0xFFFF`.

## Downstream effects

`pam_config_write_overlaps` and `pirqrc_config_write_overlaps` — the hooks the
machine layer uses to know when a configuration write needs a PAM re-attribute
or a PIRQ re-route — now use the same lane rule. A write that is not a
configuration cycle no longer reports an overlap, so it cannot trigger a
memory-attribute resync it never performed.

## Unsupported (explicit)

- Configuration Mechanism #2 (`0xCF8`/`0xCFA` CSE/forward), deleted from the
  spec for new designs and not modelled.
- Extended configuration space beyond 256 bytes (PCIe MMCONFIG / `0xCF8` bits
  27:24 as used by some chipsets).
- Software generation of Special Cycles (§3.2.2.3.3): device `0x1F`, function
  `0x7`, register 0 is treated as an ordinary absent target, not as a Special
  Cycle trigger.
- Peer host bridges and Type 1 forwarding: a non-zero bus number always
  Master-Aborts.
- Configuration retry / Master-Abort status reporting: an aborted cycle returns
  all ones but does not set Received Master Abort in any Status register.
