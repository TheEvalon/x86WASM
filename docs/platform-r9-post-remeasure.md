# SeaBIOS POST re-measure — Milestone 2 Round 9 (platform-post lane)

## Method

```text
emulator-cli --bios firmware/seabios/bios.bin --steps 2000000 --post-probe --post-spin 4096
```

Measured on branch `slice/r9-platform-post` after the four platform slices
(`PM_TMR` freerun via `tick_pit`, APM SMI halt-wake, 8042 scancode queue + host
INT 16h AH=00/01, port `0x61` ch1 refresh + speaker AND). Host used the
checked-in SeaBIOS image from the main tree (`rel-1.16.3` pin).

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

| Field | Round 5 (`merge/m2-r5-parallel-16`) | Round 9 platform-post |
|---|---|---|
| Stop PC | `F000:C897` | `F000:C897` (unchanged) |
| Busy / idle | ~1.28M busy / ~0.72M idle (~36%) | 1,281,021 busy / 718,979 idle (35%) |
| Headline | LPT + LAPIC/HPET MMIO probes | LPT/APIC/HPET presence stubs already in tree; residual unclaimed `0x3E9`/`0x2E9` |

## Platform-lane honesty

These slices improve **timer / APM / keyboard / port61** fidelity for the POST
and FreeDOS measure paths; they do **not** move SeaBIOS past `F000:C897`.

| Slice | POST-facing effect |
|---|---|
| PM_TMR via `tick_pit` | Delay loops see freerunning PM timer on every PIT quantum (not only step-clock) |
| APM SMI halt-wake | `OUT 0xB2` + `HLT` cannot wedge on the stub |
| 8042 queue + INT 16h | Host keystroke inject/check for FreeDOS measure; not a guest IVT BIOS body |
| Port 61 ch1 in `tick_pit` | Refresh-detect bit4 toggles under the same quantum POST uses |

## Still open (M2 exits)

- SeaBIOS POST complete
- FreeDOS prompt
- 32-bit Linux serial shell

No real SMM; no guest IRQ1→BDA keyboard path; no host PC-speaker audio.
