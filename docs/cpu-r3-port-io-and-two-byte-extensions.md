# CPU decode + interpreter — Milestone 2, round 3

Scope: the four bounded CPU slices landed on `slice/r3-cpu-portio`, and — just
as importantly — what they do **not** cover.

Authority: Intel SDM Vol. 1 (§3.4.1.1 register width rules, §3.6 Table 3-4
operand-size attribute, §6 stack behavior), Vol. 2 (instruction reference and
Appendix A opcode maps), Vol. 3 (§§5–6 protection and exceptions). No behavior
here is taken from another emulator.

## Shared rules

Every form added in this round obeys the size rules the earlier rounds
established:

- The operand-size and address-size attributes come from the code segment's
  cached `D` bit; `0x66` and `0x67` select the *other* size, so they invert
  under `D=1` (SDM Vol. 1 §3.6 Table 3-4; Vol. 3 §3.4.5).
- Instruction-pointer commits go through the shared `set_current_ip`, which
  writes `IP` only under `D=0` and the full `EIP` under `D=1`.
- Stack-pointer width comes from the cached `SS.B` bit, independently of the
  operand size that sizes the stack *slot* (Vol. 3 §3.4.5.1; Vol. 1 §6.2).
- Memory operands go through the shared effective-address helpers, so cached
  segment limits and the `#GP`/`#SS` split behave as in the primary map.

## Slice 1 — accumulator port I/O (`E5`, `E7`, `ED`, `EF`)

Spec: SDM Vol. 2 "IN—Input from Port", "OUT—Output to Port"; Appendix A opcode
map 1; Vol. 1 §3.4.1.1 (a 16-bit register write leaves bits 31:16 unchanged),
§3.6 Table 3-4.

Before this slice the decoder's primary table held **only** the fixed-`AL` byte
forms `E4`, `E6`, `EC`, `EE`. The accumulator forms were absent from the tables
entirely, so `OUT DX, AX` at a 16-bit operand size was as undecodable as
`OUT DX, EAX` at a 32-bit one. This is what stopped SeaBIOS POST at 17,218
retired instructions.

Supported:

- `IN eAX, imm8` (`E5`), `OUT imm8, eAX` (`E7`), `IN eAX, DX` (`ED`) and
  `OUT DX, eAX` (`EF`), each at both operand sizes. `AX` under a 16-bit operand
  size and `EAX` under a 32-bit one; `0x66` inverts the code segment's default,
  so under `CS.D=1` a bare `EF` is `OUT DX, EAX` and `66 ED` is `IN AX, DX`.
- A 16-bit `IN` writes only `AX` and leaves `EAX[31:16]` untouched.
- The `imm8` port number stays one byte at every operand size, so `E5`/`E7`
  reach only ports `0x00`–`0xFF`; the `DX` forms reach the full 16-bit space.
- All four route through the width-specific `Bus::port_in_*` / `port_out_*`
  accessors that `INSB`/`INSW`/`INSD` and `OUTSB`/`OUTSW`/`OUTSD` already use,
  so a word or doubleword port transfer is a single access of that width rather
  than a sequence of byte accesses. A test asserts the accumulator forms and the
  string forms record identical `(port, width, value)` traffic.
- No flags are affected by any form.

Not supported:

- The protected-mode I/O permission check: `#GP(0)` when `CPL > IOPL` and the
  TSS I/O-permission bitmap denies the port (SDM Vol. 1 §19.3; Vol. 3 §18.5).
  There is no TSS and execution is CPL 0 only, so the check has nothing to
  consult — it is absent rather than passing.
- The virtual-8086 mode I/O bitmap, for the same reason.
- The `#UD` a `LOCK` prefix should raise on `IN`/`OUT`.
- Any notion of port access size being rejected by the CPU; whether a device
  answers a wider access than it implements is the bus's and the device's
  business, not the interpreter's.
