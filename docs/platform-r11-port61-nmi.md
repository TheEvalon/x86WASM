# Port 0x61 NMI / parity honesty — Milestone 2 Round 11

## Why

CMOS century / UIP / alarm paths were already covered in earlier CMOS slices.
Round 9 port `0x61` work stopped at GATE2 / speaker / refresh / OUT2. FreeDOS
and some POST paths still touch System Control Port B enable and status bits
for parity / IOCHK NMI.

## Spec

- IBM PC/AT Technical Reference — System Control Port B (`0x61`):
  - bit2 enable RAM parity check
  - bit3 enable I/O channel check (IOCHK)
  - bit6 IOCHK status (read); write-1 clears
  - bit7 parity status (read); write-1 clears
- CMOS index `0x70` bit7 remains the global NMI mask

## Model (R11)

| Bit / helper | Behavior |
|---|---|
| bits 2/3 | latched enables via `port61_write` |
| bits 6/7 | host-latched status; write-1 clear |
| `Pit8254::assert_parity_error` / `assert_iochk_error` | set status; return whether enable asks for NMI |
| `Machine::inject_parity_nmi` / `inject_iochk_nmi` | latch + deliver `#NMI` if enable ∧ ¬CMOS mask |

No real DRAM parity or ISA IOCHK hardware — host/tests inject only.

## Unsupported

- Automatic parity/IOCHK generation from memory or bus errors
- NMI blocking window after delivery / SMM interaction

## Tests

- `crates/devices/src/pit.rs` — enable/status/clear/reset
- `crates/machine-pc/src/lib.rs` — `inject_parity_nmi_requires_port61_enable`
