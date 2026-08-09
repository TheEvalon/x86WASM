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

## Slice 2 — segment `PUSH`/`POP` operand-size consistency

Spec: SDM Vol. 2 "PUSH" and "POP" (Operation, segment-register source and
destination); Vol. 1 §6.2 (stack behavior, `StackAddrSize`); Vol. 3 §§3.4.5.1
(`B` flag), 5.4.1, 6.8.3.

Round 2 added `PUSH`/`POP FS`/`GS` (`0F A0`/`A1`/`A8`/`A9`) that size the stack
slot from the operand-size attribute, but left the primary-map `ES`/`CS`/`SS`/`DS`
forms (`06`/`0E`/`16`/`1E` and `07`/`17`/`1F`) always using a 16-bit slot. On a
32-bit stack that is a live corruption risk: firmware that pushes `DS` and pops
it with the two-byte encoding — or interleaves segment pushes with `PUSH`
`imm32` — would see the pointer drift by two bytes per operation.

Supported now:

- All seven primary-map forms take the slot width from the operand-size
  attribute: a word by default in a `D=0` code segment and a doubleword under
  `D=1`, with `0x66` inverting either default. The stack-pointer width still
  comes from `SS.B` independently.
- A 32-bit `PUSH` writes the **zero-extended** selector into the doubleword slot.
  The SDM permits either that or a 16-bit move that leaves the upper half of the
  slot unmodified; this model zero-extends, which is the same choice the
  `0F A0`-family already made, so the two encodings are now byte-identical. A
  test asserts that directly rather than assuming it.
- A 32-bit `POP` releases four bytes and loads the segment register from the low
  word, discarding the upper half.
- `POP SS` keeps arming the maskable-interrupt shadow, and its committed pointer
  still uses the *old* `SS.B` window because the pop happens through the old
  stack segment.
- Protected-mode `POP` still validates the descriptor through the shared
  data/stack-segment paths before either the pointer or the cache commits.

Behavior change and test impact:

This intentionally changes existing behavior. **No existing test needed
updating**, and that is itself the finding: nothing in the suite pushed or popped
a primary-map segment register at a 32-bit operand size, which is exactly why the
inconsistency survived round 2. Four new tests cover the word and doubleword
slots for push and pop, the primary-versus-two-byte agreement at both operand
sizes, and `POP SS` with a doubleword slot.

Not supported:

- Loading `SS` from the LDT, privilege-level changes, or expand-down stacks.
- The 64-bit forms (`PUSH FS`/`GS` with a 64-bit slot; `PUSH ES`/`CS`/`SS`/`DS`
  do not exist in 64-bit mode at all).
- `POP CS`, which is not an instruction — `0x0F` is the two-byte escape.

## Slice 3 — `CMOVcc` (`0F 40`–`0F 4F`)

Spec: SDM Vol. 2 "CMOVcc—Conditional Move", Appendix A opcode map 2 and
Appendix B (condition-code encodings); Vol. 1 §3.4.1.1, §3.6 Table 3-4.

Supported:

- All sixteen conditions at both operand sizes, register and memory sources.
  The condition goes through the **same** low-nibble evaluator the short `Jcc`
  (`70`+cc), near `Jcc` (`0F 80`+cc) and `SETcc` (`0F 90`+cc) families use, so
  the four families cannot disagree. A test walks all sixteen conditions against
  all thirty-two meaningful CF/PF/ZF/SF/OF combinations and compares each result
  to the short `Jcc` outcome.
- The destination is `ModR/M.reg` and its width follows the operand-size
  attribute. A taken 16-bit move leaves the upper half of the 32-bit destination
  untouched; an untaken move of either width leaves the destination entirely
  unchanged. There is no byte form.
- No flags are written.
- **The source operand is read before the condition is evaluated.** The SDM
  allows a processor to read a memory source regardless of whether the condition
  holds, so a source the segment limit or the bus rejects faults either way. A
  test asserts the `#GP` occurs with the condition both true and false, and that
  the destination is not partially committed.

CPUID interaction, stated explicitly:

`CMOVcc` is implemented but **CPUID leaf 1 EDX bit 15 (`CMOV`) stays clear**.
ADR-0007 governs CPUID and this round does not touch it. Under-reporting an
implemented feature is safe under the truthful-CPUID rule — the risk the rule
guards against is the opposite direction. Software that gates `CMOVcc` on the
feature bit will simply take its non-`CMOV` path. Setting the bit belongs with a
CPUID revision, not with an instruction slice.

Not supported:

- The REX.W `r64` destination form.
- 64-bit mode's rule that a 32-bit operand size zero-extends the destination even
  when the condition is false; that rule has no 16/32-bit analogue and long mode
  is out of scope.
- The `#UD` a `LOCK` prefix should raise.

## Slice 4 — `SHLD`/`SHRD` (`0F A4`/`A5`/`AC`/`AD`)

Spec: SDM Vol. 2 "SHLD—Double Precision Shift Left" and "SHRD—Double Precision
Shift Right" (Description, Operation, Flags Affected); Appendix A opcode map 2.

Supported:

- All four encodings — `imm8` count (`0F A4`/`0F AC`) and `CL` count
  (`0F A5`/`0F AD`) — at both operand sizes, with a register or memory
  destination. The destination is `r/m`, the bits shifted in come from
  `ModR/M.reg`, and the source register is never modified.
- **Count masking.** `COUNT := COUNT MOD 32` outside 64-bit mode,
  *independently of the operand size*. That is a real subtlety: the mask is 32
  even when the operand size is 16, so a 16-bit `SHLD` can legally receive a
  count of 17–31 — which is precisely the "bad parameters" case below. A count
  that masks to zero is an explicit no-operation with no flag change at all.
- **Count equal to the operand size** is defined and is *not* the bad-parameter
  case: every destination bit comes from the source, and `CF` is still the last
  destination bit shifted out (`BIT[DEST, 0]` for `SHLD`, `BIT[DEST, SIZE-1]`
  for `SHRD`).
- `CF` is the last bit shifted out of the destination: `BIT[DEST, SIZE-COUNT]`
  for `SHLD` and `BIT[DEST, COUNT-1]` for `SHRD`. `SF`, `ZF` and `PF` follow the
  result through the shared shift-result helpers.
- The destination is written *before* any flag commits, so a faulting memory
  write leaves the flags untouched.

Deterministic choices where the SDM says "undefined". The interpreter is the
semantic reference for a future JIT, so an indeterminate result would be
untestable; each of these is a legal instance of the undefined behavior, and each
has a test that pins it:

- **Count greater than the operand size** ("Bad parameters" in the SDM Operation
  section) leaves the destination *and* all six flags unchanged, and emits no
  destination write. Reachable only at a 16-bit operand size, since the
  modulo-32 mask caps a 32-bit count at 31.
- **`OF` above a 1-bit shift** is left unchanged. For a 1-bit shift it is set
  when the sign of the destination changed, which is the defined rule. Leaving it
  alone for larger counts is the same choice the Group 2 shifts
  (`ROL`/`ROR`/`RCL`/`RCR`/`SHL`/`SHR`/`SAR`) already make in this tree, so the
  two shift families agree.
- **`AF`**, undefined in every case, is left unchanged — again matching the
  Group 2 shifts.

Not supported:

- The REX.W 64-bit forms (and therefore the `COUNT MOD 64` mask).
- The `#UD` a `LOCK` prefix should raise.

## What POST reached, and the findings past this round

`cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin --post-probe
--steps 2000000` progressed as follows, one entry per slice:

| After | Steps | Stop |
|---|---|---|
| baseline (`0195f78`) | 17,218 | unsupported opcode `0xEF` at `0008:417B` |
| slice 1 (port I/O) | 50,511 | unsupported opcode `0x0F 0xAC` at `0008:CDDB` |
| slice 2 (segment stack slots) | 50,511 | unchanged |
| slice 3 (`CMOVcc`) | 50,511 | unchanged |
| slice 4 (`SHLD`/`SHRD`) | 2,000,000 | **step budget exhausted — no unsupported opcode at all** |

**There is no longer a CPU decode blocker on SeaBIOS's POST path.** The stop is a
step budget, and re-running at 50,000,000 steps produces a byte-identical report:
the same unmapped-MMIO pages with the same counts, no new page touched, no port
`0x80` checkpoint, and nothing on COM1 or the `0x402` debug console. SeaBIOS is
spinning in a loop over memory it has already touched.

Two findings, both **outside this area's ownership**, recorded and deliberately
not fixed:

1. **Linear addresses are not wrapped modulo 2^32.** The probe logs unmapped
   MMIO at `0x1_000D5000` (86 writes, 10 reads) and `0x1_000FF000` (4 writes,
   42 reads) — above the 4 GiB boundary. `x86_mmu::linear_addr` is
   `seg.base.wrapping_add(offset)` in `u64` with no 32-bit truncation, so a
   segment base near the top of the address space plus a large offset escapes the
   4 GiB linear space instead of wrapping into low memory. Masked to 32 bits
   those two pages are `0x000D5000` and `0x000FF000`, both plausible low-memory
   targets — the second is the top of the `0xF0000`–`0xFFFFF` BIOS segment
   SeaBIOS uses for its f-segment data. Outside 64-bit mode the linear address
   space is 4 GiB and the sum wraps (SDM Vol. 3 §3.3.1). The fix belongs in
   `crates/x86-mmu`.
2. **A 64 KiB write sweep at `0xF0000000`–`0xF000FFFF`** that no RAM or ROM
   window covers, walking downward one page at a time (4,096 writes per page,
   2,048 in the top page). Not explained here; it is either a memory-map gap in
   `machine-pc`/`devices` or a second consequence of finding 1. Recorded with
   exact page counts so whoever owns the memory map can start from data.

The probe reports no `CS:IP`/`EIP` when it stops on the step budget, which is now
the limiting factor on diagnosing further. Adding the current PC (and ideally a
short trailing PC histogram) to `PostStopReason::StepBudgetExhausted` would turn
"it spins" into "it spins here"; that is a `machine-pc` change.

## Interaction with `docs/cpu-0f-map.md`

That document's "Remaining unimplemented `0F` opcodes" list still names
`0F 40`–`0F 4F` (`CMOVcc`) and `0F A4`/`A5`/`AC`/`AD` (`SHLD`/`SHRD`), which this
round implements, and its slice-2 section still records the primary-map segment
`PUSH`/`POP` inconsistency as "a round-3 item". Updating it is an integration
step: this area's ownership covers only new `docs/cpu-r3-*.md` files.
