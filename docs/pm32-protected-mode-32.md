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
still use 16-bit `SP`; 32-bit IDT gates and `IRETD`; delivering an
architectural fault, software interrupt, NMI, or IRQ while `CS.D=1` (the
bounded 16-bit gate path reports
`ProtectedModeExceptionDelivery { reason: CurrentPrivilege }` instead of
truncating a 32-bit return `EIP` into a 16-bit frame); protected far `CALL`,
call gates, tasks; conforming segments, privilege switching, or outer-level
returns; branch-time `CS`-limit `#GP` (a target beyond the limit is reported by
the next instruction fetch); `L=1` / 64-bit defaults.
