# Port 0x61 speaker / refresh polish — Milestone 2 Round 9

## Why

SeaBIOS POST programs PIT channel 1 for DRAM refresh and polls System Control
Port B bit4 until it toggles. Channel 2 + port `0x61` bits 0/1/5 form the PC
speaker path used for the POST beep. Round-5 already toggled bit4 from
[`Pit8254::tick_ch1`], but [`Machine::tick_pit`] only advanced ch0/ch2 — so the
step clock never moved refresh-detect during `--post-probe`.

## Spec

- Intel 8254 — channel 1 mode 2 rate generator (IBM PC/AT DRAM refresh);
  channel 2 speaker timer gated by port `0x61` bit0.
- IBM PC/AT System Control Port B (`0x61`):
  - bit0 GATE2 (writable)
  - bit1 speaker data enable (writable)
  - bit4 refresh-detect (read-only; toggles on ch1 OUT rising edges)
  - bit5 OUT2 (read-only channel-2 OUT)
- PC speaker drive is the AND of GATE2, speaker data, and OUT2 (no host audio
  here — digital enable only).

## Model (R9)

[`Machine::tick_pit`] advances **ch0 + ch1 + ch2** (plus PM_TMR ×3):

| Bit / helper | Source |
|---|---|
| bit4 refresh-detect | ch1 rising OUT via `tick_ch1` inside `tick_pit` |
| bit5 OUT2 | ch2 OUT via `tick_ch2` |
| [`Pit8254::speaker_output_enabled`] | GATE2 ∧ SPKR_DATA ∧ OUT2 |

Guest writes still cannot overwrite bit4/bit5.

## Unsupported

- No host PC-speaker audio waveform.
- No DRAM refresh bus-cycle side effects beyond the ch1 OUT / bit4 toggle.
- NMI / parity bits on port `0x61` remain unimplemented.

## Tests

- `crates/devices/src/pit.rs` — speaker AND path on existing port61 test.
- `crates/machine-pc/tests/port61_refresh.rs` — `tick_pit` toggles bit4.
