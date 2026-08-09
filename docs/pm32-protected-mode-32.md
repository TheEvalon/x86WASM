# Same-CPL 32-bit protected mode (Milestone 2, round 1)

Scope notes for the bounded move from the existing 16-bit same-CPL
protected-mode path to 32-bit (`D`/`B` = 1) same-CPL, ring-0 protected mode.

Everything here is **CPL 0 â†’ CPL 0 only**. Privilege switching, call gates,
TSS/task gates, LDT resolution, and paging are explicitly out of scope and
belong to later rounds.

## Authoritative sources

- Intel SDM Vol. 1 Â§3.5 (EIP), Â§3.6 + Table 3-4 (operand-size / address-size
  attributes), Â§6.5 (procedure calls, stack frames)
- Intel SDM Vol. 2 Chapter 2 (instruction format, `66H` / `67H` overrides);
  `JMP`, `CALL`, `RET`, `Jcc`, `LOOP`, `PUSH`, `POP`, `PUSHA(D)`, `POPA(D)`,
  `PUSHF(D)`, `POPF(D)`, `ENTER`, `LEAVE`, `INT n`, `IRET/IRETD`
- Intel SDM Vol. 3 Chapter 5 (Protection): Â§5.3 limit checking, Â§5.8.1 direct
  far transfers
- Intel SDM Vol. 3 Chapter 6 (Interrupt and Exception Handling): Â§6.11
  (IDT descriptors), Â§6.12.1 (interrupt/trap gate delivery), Â§6.13 (error
  codes), Â§6.14 (exception/interrupt reference)

## Slice 1 â€” `CS.D=1` default-32 execution

**Supported.** `x86-decode` gained a `DecodeMode` describing the code-segment
default operand-size and address-size attributes plus `decode_with_mode`.
`DecodedInsn` now reports the *effective* `operand_size_32` /
`address_size_32` after applying `66H` / `67H` to those defaults, so the
override prefixes select 16-bit forms when `CS.D=1`. This covers immediate,
displacement, `rel16`/`rel32`, `moffs`, and `ptr16:16`/`ptr16:32` widths, and
selects the 32-bit ModR/M + SIB addressing forms without `67H`.

The interpreter resolves both attributes from the decoded instruction, so every
already-implemented ALU/MOV/stack/string/group form follows `CS.D`. Instruction
fetch and the sequential instruction-pointer advance now run in a CS.D-selected
window: `D=0` keeps the legacy 16-bit `IP` wrap (bit-identical to the previous
behavior), `D=1` uses the full 32-bit `EIP`. Near `JMP`/`Jcc`/`LOOP`/`JCXZ`/
`CALL`/`RET` targets follow the SDM rule that a 16-bit operand size clears
`EIP[31:16]`; a 32-bit operand size keeps all 32 bits.

Direct protected far `JMP` (`EA ptr16:16`, `EA ptr16:32`, `FF /5 m16:16`,
`FF /5 m16:32`) now accepts a present, nonconforming, ring-0 GDT code segment
with either `D=0` or `D=1`, loading the full cached attribute set (access byte
plus AVL / L / D-B / G) so entering and leaving 32-bit code works. `L=1`
targets are still rejected.

**Not supported by this slice.** 32-bit stacks (`SS.B=1`) â€” pushes and pops
still use 16-bit `SP` (closed by slice 2); 32-bit IDT gates and `IRETD`;
delivering an
architectural fault, software interrupt, NMI, or IRQ while `CS.D=1` (the
bounded 16-bit gate path reports
`ProtectedModeExceptionDelivery { reason: CurrentPrivilege }` instead of
truncating a 32-bit return `EIP` into a 16-bit frame); protected far `CALL`,
call gates, tasks; conforming segments, privilege switching, or outer-level
returns; branch-time `CS`-limit `#GP` (a target beyond the limit is reported by
the next instruction fetch); `L=1` / 64-bit defaults.

## Slice 2 — `SS.B=1` 32-bit stacks

**Supported.** The stack-pointer helpers now derive the stack address size
from the cached `SS.B` bit (SDM Vol. 3 §3.4.5.1, Vol. 1 §6.2.2). With `B=1`
every push and pop uses the full 32-bit `ESP` and wraps modulo 2^32; with
`B=0` only `SP` changes, `ESP[31:16]` is preserved, and the pointer wraps
modulo 2^16 exactly as before. The pushed/popped *width* still follows the
operand-size attribute, so `CS.D=1` code defaults to dword slots.

This covers `PUSH`/`POP` of registers, immediates, `r/m` memory and segment
registers, `PUSHF`/`PUSHFD` and `POPF`/`POPFD`, `PUSHA`/`PUSHAD` and
`POPA`/`POPAD` (including the `Temp` slot, which is `ESP` before the first
push when `B=1`), near `CALL`/`RET`, the `RET`/`RETF imm16` stack release,
`ENTER`/`ENTERD` (level 0 and nested display walks) and `LEAVE`. The stack
segment limit is checked against the stepped pointer before it is committed,
so a push that wraps outside the limit raises `#SS(0)` atomically.

`POP SS` commits its pointer update with the *old* `SS.B`, because the pop
itself happens through the old stack segment.

**Closed `Unsupported` gap.** `ENTER`, `LEAVE`, `PUSHA(D)` and `POPA(D)` used
to return `Unsupported` when prefixed with `0x67`. Per SDM Vol. 1 §6.2.2 the
address-size override applies to memory operands and does not change the
stack address size, so those forms now execute and take their pointer width
from `SS.B`. The previous `*_asize32_unsupported` tests were replaced with
tests asserting that behavior.

**Model note.** For `ENTER`/`LEAVE`, this tree uses the operand-size attribute
for the pushed/popped width *and* for the `BP` vs `EBP` frame-pointer register,
while `SS.B` selects `SP` vs `ESP`. The SDM `ENTER` pseudocode expresses both
in terms of `StackSize`; the split above preserves the previously tested
`ENTERD`-on-a-16-bit-stack behavior and matches the operand-size-selected
`ENTER`/`ENTERD` mnemonics.

**Not supported by this slice.** 64-bit stacks (`RSP`); 32-bit IDT gates
(delivery still requires a 16-bit stack and reports
`ProtectedModeDeliveryError::StackWidth` otherwise); `IRETD`; `IRET16` on a
`B=1` stack; privilege-level stack switching, TSS `SS0:ESP0`, or expand-down
stack segments.

## Slice 3 — 32-bit IDT gates

**Supported.** `deliver_protected_mode_gate` now accepts 386 gate types
`0xE` (interrupt gate) and `0xF` (trap gate) alongside the existing 286 types
`0x6` and `0x7`. For a 386 gate the entry `EIP` is assembled from the gate
offset low word (bytes 1:0) and high word (bytes 7:6), and the frame is built
from doublewords: `EFLAGS`, `CS` zero-extended to 32 bits, the return `EIP`,
and — for exceptions that define one — a doubleword error code. Interrupt
gates clear `IF`; trap gates preserve it. Both clear `TF`, `NT`, `RF`, and
`VM`.

The frame element width comes from the gate type; the stack-pointer width
comes from the cached `SS.B` bit, so a 386 gate works on both a `B=1` and a
`B=0` stack. A 386 gate may be taken from `CS.D=0` or `CS.D=1` code, and may
target a `D=0` or `D=1` ring-0 nonconforming code segment. A 286 gate still
requires both the current and the target code segment to be `D=0` and reports
`CurrentPrivilege` / `TargetNot16Bit` otherwise, rather than truncating a
32-bit return `EIP` into a 16-bit frame. `L=1` targets report the new
`TargetLongMode` reason.

Gate DPL is still checked only for software `INT n` / `INT3` / taken `INTO`
(violation ? `#GP((vector << 3) | IDT)` with the stack untouched) and ignored
for NMI and external IRQ. Delivery remains atomic: the gate, the target
descriptor, the offset-vs-limit check, and every stack address are validated
before any write, and a failing stack write rolls all bytes back.

**Not supported by this slice.** `IRETD` (slice 4); privilege-level changes,
so no stack switch, no TSS `SS0:ESP0`, and no outer-level frame with `SS:ESP`;
task gates; interrupt/trap gates in the LDT; virtual-8086 delivery; nested
`#DF` or triple-fault synthesis; the `IST` mechanism.

## Slice 4 — `IRETD`

**Supported.** `protected_iret16` became `protected_iret`, driven by the
effective operand size: `IRETD` (`CS.D=1` default, or `0x66` from `D=0` code)
pops a 12-byte `EIP`/`CS`/`EFLAGS` frame, `IRET` pops the existing 6-byte
`IP`/`CS`/`FLAGS` frame. The stack-pointer width follows `SS.B`, so both frame
sizes work on 16-bit and 32-bit stacks and the pointer is advanced by 12 or 6
through `ESP` or `SP` respectively.

The frame and the return descriptor are read and fully validated before any
architectural commit. The return code segment must be a non-null, GDT,
non-system, executable, nonconforming, present, ring-0 (`RPL=DPL=0`) segment
with `L=0`; `D=0` and `D=1` are both accepted and the reloaded CS cache keeps
the access byte plus the AVL, D/B, and G attributes, so `IRETD` can switch the
execution window in either direction. A return `EIP` beyond the effective
segment limit raises `#GP(0)`; selector problems raise `#GP(selector)` or
`#NP(selector)`.

Flag restore at CPL 0: a 16-bit return restores `FLAGS[15:0]` (mask `0x7FD5`)
and leaves `RFLAGS[63:16]` unchanged; a 32-bit return restores `EFLAGS`
through `ID` (mask `0x003D_7FD5`: CF, PF, AF, ZF, SF, TF, IF, DF, OF, IOPL,
NT, RF, AC, VIF, VIP, ID) and leaves `RFLAGS[63:32]` unchanged. Reserved bits
3, 5, and 15 stay clear and bit 1 stays set in both cases.

A round-trip test enters a 386 interrupt gate and returns with `IRETD`,
asserting that the full `CpuState` (except the saved next `EIP`) matches the
pre-interrupt state.

**Not supported by this slice.** Outer-level returns (no `SS:ESP` pop, no
privilege change); returns to virtual-8086 mode — `VM=1` in the popped image
is reported as `Unsupported(0xCF)` rather than silently ignored; nested task
returns (`NT=1` in the current `EFLAGS`) are likewise `Unsupported`;
conforming return segments; LDT return selectors; real-address-mode `IRETD`
(`0x66 CF` with `CR0.PE=0`) still pops the 6-byte real-mode frame; `IRETQ`.
