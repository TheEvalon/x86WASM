# SeaBIOS POST re-measure — Milestone 2 Round 11 (platform-post lane)

## Method

```text
emulator-cli --bios firmware/seabios/bios.bin --steps 2000000 --post-probe --post-spin 4096
```

Measured on branch `slice/r11-platform-post` after the four platform slices
(IRQ1→BDA keyboard, APM INT 15h AH=53h stub, this remeasure, port `0x61`
parity/IOCHK NMI). Host used the checked-in SeaBIOS image
(`firmware/seabios/bios.bin`, same pin as R9).

## Result (2026-08-10)

```text
post-probe: steps=1281021 stop=step-budget-exhausted
  stop-pc        cs:ip=F000:C897 cs.d=0 eip=0x0000C897 linear_pc=0x00000000000FC897 bytes=[FA FC 66 C3 66 55 66 57]
  halt-idle      idle-steps=718979 busy-steps=1281021 idle-pct=35%
  spin           sampled=4096 window=4096 distinct=399 cycle=none
  unclaimed-port out/in port=0x03E9 and 0x02E9 (once each)
  post-codes=[] last=none
  com1="" debug=""
```

## Comparison

| Field | Round 9 platform-post | Round 11 platform-post |
|---|---|---|
| Stop PC | `F000:C897` | `F000:C897` (**unchanged**) |
| Busy / idle | 1,281,021 busy / 718,979 idle (35%) | identical |
| Unclaimed | `0x3E9` / `0x2E9` | same (COM3/COM4 IER probe sites) |

## Platform-lane honesty

These slices improve **keyboard BDA / APM BIOS check / port61 NMI** fidelity for
the FreeDOS measure path; they do **not** move SeaBIOS past `F000:C897`.

| Slice | POST-facing effect |
|---|---|
| IRQ1 → BDA `40:1E` | Host keyboard ring for FreeDOS; not a guest INT 09h body |
| APM INT 15h AH=53h | Host installation/connect stub; **not** real SMM |
| Port 61 parity/IOCHK | Enable+status bits honest; no automatic parity hardware |
| This remeasure | Confirms stop PC vs R9 |

## Residual gaps (do **not** claim POST complete)

- SeaBIOS still exhausts the step budget at `F000:C897` (`CLI`/`CLD`/`RETF`-shaped
  window) — same first-contact stop as R5/R9.
- Unclaimed `0x3E9`/`0x2E9` (not LPT; open-bus COM3/COM4 IER probes).
- No POST checkpoint codes on port `0x80`; empty COM1/debug consoles.
- Still open M2 exits: SeaBIOS POST complete, FreeDOS prompt, 32-bit Linux
  serial shell.

## Still unsupported

- Real SMM / SMBASE relocate
- Guest INT 09h BIOS body
- Automatic DRAM parity / ISA IOCHK generation
