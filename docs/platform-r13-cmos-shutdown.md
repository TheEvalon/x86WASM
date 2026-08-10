# CMOS shutdown status `0Fh` polish — Milestone 2 Round 13 (platform-io)

## Why

SeaBIOS soft-reset / `qemu_reboot` writes CMOS shutdown status before pulsing
ICH `0xCF9`. The byte was already battery-backed store/readback; R13 adds
Machine helpers and documents that pulse-reset preserves it without dispatching
POST resume actions.

## Spec

- IBM PC/AT CMOS map index `0x0F` — shutdown status / reset code (ordinary
  battery CMOS RAM, not MC146818 status A–D).
- Ralf Brown's Interrupt List CMOS 0Fh — reset-code table (`00h` soft/unexpected,
  `01h`–`03h` memory POST, `04h` INT 19h, `05h` EOI+JMP, `09h` block move,
  `0Ah` JMP via BDA `40:67`, `0Bh` IRET, `0Ch` RETF, …).

## Model

- `CmosRtc::set_shutdown_status` / `shutdown_status`
- `Machine::set_shutdown_status` / `shutdown_status`
- Survives `CmosRtc::reset` and therefore `Machine::reset` / CF9 pulse-reset
- Named constants: `SHUTDOWN_AFTER_MEM_*`, `SHUTDOWN_IRET`, `SHUTDOWN_RETF`, …
- **No** Machine dispatch on the code (no JMP via `40:67`, no auto INT 19h)

## Unsupported

- Firmware soft-reset resume path driven by shutdown code
- Clearing the byte automatically after POST consumes it
