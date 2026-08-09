# Same-CPL 32-bit protected mode (Milestone 2, round 1)

Scope notes for the bounded move from the existing 16-bit same-CPL
protected-mode path to 32-bit (`D`/`B` = 1) same-CPL, ring-0 protected mode.

Everything here is **CPL 0 → CPL 0 only**. Privilege switching, call gates,
TSS/task gates, LDT resolution, and paging are explicitly out of scope and
belong to later rounds.

## Authoritative sources

- Intel SDM Vol. 1 §3.5 (EIP), §3.6 + Table 3-4 (operand-size / address-size
  attributes), §6.5 (procedure calls, stack frames)
- Intel SDM Vol. 2 Chapter 2 (instruction format, `66H` / `67H` overrides);
  `JMP`, `CALL`, `RET`, `Jcc`, `LOOP`, `PUSH`, `POP`, `PUSHA(D)`, `POPA(D)`,
  `PUSHF(D)`, `POPF(D)`, `ENTER`, `LEAVE`, `INT n`, `IRET/IRETD`
- Intel SDM Vol. 3 Chapter 5 (Protection): §5.3 limit checking, §5.8.1 direct
  far transfers
- Intel SDM Vol. 3 Chapter 6 (Interrupt and Exception Handling): §6.11
  (IDT descriptors), §6.12.1 (interrupt/trap gate delivery), §6.13 (error
  codes), §6.14 (exception/interrupt reference)

## Slice 1 — `CS.D=1` default-32 execution

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

**Not supported by this slice.** 32-bit stacks (`SS.B=1`) — pushes and pops
still use 16-bit `SP` (closed by slice 2); 32-bit IDT gates and `IRETD`;
delivering an
architectural fault, software interrupt, NMI, or IRQ while `CS.D=1` (the
bounded 16-bit gate path reports
`ProtectedModeExceptionDelivery { reason: CurrentPrivilege }` instead of
truncating a 32-bit return `EIP` into a 16-bit frame); protected far `CALL`,
call gates, tasks; conforming segments, privilege switching, or outer-level
returns; branch-time `CS`-limit `#GP` (a target beyond the limit is reported by
the next instruction fetch); `L=1` / 64-bit defaults.

## Slice 2 � `SS.B=1` 32-bit stacks

**Supported.** The stack-pointer helpers now derive the stack address size
from the cached `SS.B` bit (SDM Vol. 3 �3.4.5.1, Vol. 1 �6.2.2). With `B=1`
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
to return `Unsupported` when prefixed with `0x67`. Per SDM Vol. 1 �6.2.2 the
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
