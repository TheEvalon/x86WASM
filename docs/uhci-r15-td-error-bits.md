# UHCI TD short-packet / Stall honesty — Milestone 2 Round 15

## Why

R8–R14 one-TD walks cleared Active and wrote Actual Length as if every transfer
succeeded. Firmware error paths need Stall + USBERRINT, and short IN packets
must not look like a full MaxLen success.

## Spec

- UHCI 1.1 §3.2.2 — TD Control/Status
  - bit 23 Active
  - bit 22 Stalled
  - bit 29 Short Packet Detect (SPD) enable (presence; completion via ActLen)
  - bits 10:0 Actual Length (`n−1` encoding)
- UHCI 1.1 §2.1.2 — USBERRINT on transaction error

## Model

| Outcome | Active | Stalled | ActLen | USBERRINT | Notes |
|---|---|---|---|---|---|
| Full success | 0 | 0 | = MaxLen | no | unchanged |
| IN short packet | 0 | 0 | actual | no | `short_packet=true`; SPD bit retained if set |
| Stall (`run_one_td_stall` or OUT short buf) | 0 | 1 | 0 | yes | not a fake success |
| Zero-length IN | 0 | 0 | 0 (`0x7FF`) | no | short packet |

OUT/SETUP with a device buffer shorter than MaxLen completes as **Stall**
rather than a fake short success.

## Not wired (explicit)

- NAK retry counters / CRC/Timeout/Babble distinct bits
- QH halt-on-SPD schedule semantics beyond TD status
- Real USB device stack

## Tests

- `td_in_short_packet_sets_actlen_not_stall`
- `td_stall_sets_stalled_and_usberrint`
- `td_out_short_device_buf_stalls`
