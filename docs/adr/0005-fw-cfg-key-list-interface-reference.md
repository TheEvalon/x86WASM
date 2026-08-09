# ADR-0005: QEMU/SeaBIOS fw_cfg headers are an interface reference, not source

- Status: Accepted
- Date: 2026-08-09

## Context

`AGENTS.md` and `.cursor/rules/emulator-core.mdc` prohibit copying implementation
code from v86, QEMU, Bochs, VirtualBox, DOSBox, and other emulators. That rule
is not negotiable and this ADR does not weaken it.

It does, however, leave a real question unanswered, and successive agents have
had to guess at the answer. This emulator exists to boot SeaBIOS. SeaBIOS talks
to the platform through fw_cfg, an interface whose key numbers, field widths,
and blob layouts are facts about the wire protocol, not about anyone's
implementation of it. The published QEMU fw_cfg *specification document* is
already an approved source in `docs/sources.md`, but it does not enumerate every
key: the authoritative list of key numbers lives in QEMU's `hw/i386/fw_cfg.h`
and `include/standard-headers/`, and the guest-side view of the same numbers
lives in SeaBIOS's own headers.

Without a stated boundary, an agent has two bad options: invent key numbers
(which cannot interoperate and violates "never invent behavior"), or read the
headers and be unsure whether it just broke the no-copying rule.

## Decision

**QEMU's `fw_cfg.h` and SeaBIOS's corresponding headers are approved as an
interface reference only.**

Approved to read and use:

- fw_cfg key numbers and their names.
- Field widths, byte order, and struct layout of fw_cfg blobs and of the DMA
  access structure.
- Firmware file names (`etc/e820`, and future `etc/*` entries) and the layout of
  their contents.
- Constants whose value *is* the interface, such as the 56-byte file-name field.

Not approved, unchanged by this ADR:

- Any function body, state machine, control flow, error handling, or data
  structure that implements the interface.
- Copying header text verbatim into this repository. The facts may be restated;
  the file may not be reproduced.
- Extending the same treatment to any other part of QEMU, SeaBIOS, or another
  emulator. This ADR covers fw_cfg interface definitions and nothing else.

The test to apply: *if two independent implementations must agree on this value
to interoperate, it is an interface fact.* If it is a choice one implementation
made about how to do the work, it is implementation and stays off limits.

Every constant taken this way must still be cited in `docs/sources.md` and
carry a spec comment at its definition, exactly like a constant taken from the
Intel SDM.

## Consequences

Easier: the fw_cfg device can be completed against the real key list rather than
a partial one, and future agents have a written answer instead of re-litigating
the question every round.

Harder, deliberately: the boundary is narrow and needs judgement at the edge.
An agent that finds itself reading past a `#define` block into a function is
outside the approval and must stop. Reviewers should treat any fw_cfg change
whose diff resembles upstream control flow as a red flag regardless of this ADR.

Residual risk: interface facts and implementation choices are not always cleanly
separated in a header. Where a value looks like a QEMU-specific policy rather
than a protocol requirement, prefer to leave the behavior unimplemented and
record the gap over guessing which side of the line it falls on.
