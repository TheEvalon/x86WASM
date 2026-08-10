# APM/SMI handshake deepen — Milestone 2 Round 9

## Why

Round 5 cleared APM_STS on every APM_CNT write so SeaBIOS
`smm_relocate_and_restore` could exit its status poll. Residual wedge: SeaBIOS
`call32_smm` does `OUT 0xB2` then `HLT` waiting for an architectural SMI. With
`IF=0` and no SMI, `--post-probe` treated that as a permanent halt.

## Spec

- Intel PCH/ICH fixed I/O: APM_CNT (`B2h`) / APM_STS (`B3h`).
- Intel SDM Vol. 2 — `HLT`: SMI resumes a halted processor.
- SeaBIOS `smm_relocate_and_restore` (poll status until 0) and optional
  `call32_smm` (OUT then HLT).

## Model (R9)

| Event | Behavior |
|---|---|
| Write APM_CNT | store command, clear APM_STS, `stub_completions++`, latch `smi_pending` |
| Read APM_STS when 0 | consume `smi_pending` (poll path done; no later false wake) |
| `Machine::step` after `HLT` with pending | clear `halted`, `smi_wake_stubs++`, clear pending |

Helpers: [`ApmSmi::service_halt_wake`], [`Machine::service_apm_smi_halt_wake`].

## Still unsupported (explicit)

- **No real SMM** — no SMBASE relocate, no SMRAM entry, no RSM, no EIP rewrite.
- `call32_smm` still cannot trampoline to 32-bit SeaBIOS code; it only avoids
  wedging on the post-OUT `HLT`.
- APMC_EN / SMI_EN / GLBCTL gating is not checked.

## Tests

- `crates/devices/src/apm.rs` — poll consume, halt wake, reset.
- `crates/machine-pc/tests/apm_smi_ports.rs` — bus probe + OUT+HLT resume.
