# APM/SMI ports `0xB2`/`0xB3` — Milestone 2 Round 5

## Why

SeaBIOS POST spun at `0008:25C2` (`TEST AL,AL` / `JNZ`) after:

```text
OUT 0xB3, 0x01
OUT 0xB2, 0x00
IN  0xB3   ; open-bus 0xFF forever
```

That is SeaBIOS `smm_relocate_and_restore`: set APM status busy, raise SMI
via the APM control port, poll until the SMI handler clears status.

## Spec

- Intel PCH/ICH fixed I/O: **APM_CNT** (`B2h`) and **APM_STS** (`B3h`) — 8-bit
  R/W, default `00h`. A write to APM_CNT stores the command and raises SMI#
  when APMC_EN is set in the chipset SMI enable path. A write to APM_STS does
  not raise SMI#; it is the APM BIOS 1.2 / PIIX software scratchpad between
  the guest and the SMI handler.
- SeaBIOS (rel-1.16.3) `src/fw/smm.c` `smm_relocate_and_restore` +
  `handle_smi` (handler clears status with `outb(0x00, PORT_SMI_STATUS)`).

## Model

`devices::ApmSmi` + `MachineBus` decode:

| Port | Read | Write |
|---|---|---|
| `0xB2` APM_CNT | last command | store command, then **stub-complete** (clear `0xB3` to `0`) |
| `0xB3` APM_STS | status | store scratchpad (no SMI) |

`stub_completions` counts APM_CNT writes that took the stub path.

## Unsupported (explicit)

- No architectural SMI delivery, no SMM entry, no SMBASE relocation, no RSM.
- APMC_EN / SMI_EN / GLBCTL gating is not checked; every APM_CNT write
  stub-completes.
- APM BIOS 1.2 power-state functions beyond this handshake are out of scope.
- Later SeaBIOS `call32_smm` paths that `OUT 0xB2` then `HLT` waiting for a
  real SMI still need SMM (recorded when the probe reaches them).

## Tests

- `crates/devices/src/apm.rs` unit tests (defaults, scratchpad, handshake).
- `crates/machine-pc/tests/apm_smi_ports.rs` bus + probe claim coverage.
