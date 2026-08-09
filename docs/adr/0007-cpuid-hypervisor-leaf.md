# ADR-0007: Do not fabricate a CPUID hypervisor signature at `0x4000_0000`

- Status: Accepted
- Date: 2026-08-09

## Context

Round 2 implemented `CPUID`. SeaBIOS probes leaf `0x4000_0000` for a hypervisor
signature and sets its internal `runningOnQEMU` flag when it finds `TCGTCGTCGTCG`
or `KVMKVMKVM`. This machine returns no signature, so the flag stays false.

That is not a neutral outcome. `runningOnQEMU` gates a substantial amount of
SeaBIOS's platform behavior: which interfaces it trusts, which enumeration paths
it takes, and which shortcuts it allows itself. A false value may send POST down
a path this emulator's device set does not support, and the failure would show
up somewhere far from the CPUID call that caused it. The tempting fix is to
answer the probe with a QEMU signature and move on.

There is also a rule to reconcile. `AGENTS.md` requires truthful CPUID: never
advertise an unimplemented feature. It is worth being precise about whether a
hypervisor signature is covered by that rule, because the answer is not
obviously yes.

## Decision

**Do not return a hypervisor signature at leaf `0x4000_0000` yet.** The leaf
stays absent and `runningOnQEMU` stays false.

The reasoning, recorded so it is not re-derived:

1. A hypervisor signature is a **platform identity** claim, not a CPU feature
   claim. The truthful-CPUID rule targets feature bits, where advertising a bit
   means "software may execute this instruction and it will work". A vendor
   string at `0x4000_0000` says "this platform is X", which is a different kind
   of statement. So returning one would not violate the truthful-CPUID rule per
   se, and this ADR does not want future agents to believe it is forbidden on
   those grounds.

2. It is still not free. Claiming to be QEMU asks firmware to assume a whole
   platform contract — device set, fw_cfg content, ACPI tables, PCI layout —
   most of which this emulator does not yet satisfy. A false platform identity
   converts a clean "unsupported" into misdirected firmware and much harder
   debugging.

3. Therefore the bar is **measurement, not convenience**. If the leaf is added,
   it must be because a measurement showed the absent signature is what blocks
   or misdirects POST — not because it seemed likely to help.

### Trigger condition

Revisit this decision when the POST probe shows SeaBIOS blocked or misdirected
by `runningOnQEMU == false`. Concretely: a probe stop, a wrong branch, or a
skipped initialization that traces back to that flag. At that point, decide with
the measurement in hand, and re-examine which platform contract the chosen
signature implies before returning it.

Until then this stays a known, recorded divergence rather than a bug.

## Consequences

Easier: POST failures stay attributable. A machine that does not claim to be
QEMU gets SeaBIOS's generic path, and when that path fails it fails for a
reason this tree can act on.

Harder: some SeaBIOS behavior will differ from every reference run of the same
binary under QEMU, so differential comparison against a QEMU run is not
apples-to-apples on any code path `runningOnQEMU` guards. Anyone using QEMU as
an oracle needs to know that before trusting a diff.

Related: ADR-0005 draws the same kind of line for fw_cfg — interoperating with
firmware is legitimate, impersonating a platform this emulator is not is a
separate decision that needs its own justification.
