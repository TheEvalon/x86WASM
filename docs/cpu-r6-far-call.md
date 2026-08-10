# Round 6 — same-CPL protected far CALL

## Choice

Slice 4 preferred **same-CPL far `CALL` to a GDT code segment** (`9A` /
`FF /3`) over `VERR`/`VERW` / call-gate `CALL` because R5 already provided
direct far `JMP` descriptor validation, and call gates need more gate-type
infrastructure. `VERR`/`VERW` remain deferred; they can reuse the LAR/LSL soft
checks when needed.

## Scope

- Protected-mode far `CALL ptr16:16` / `ptr16:32` and `FF /3` memory-indirect
- Same-CPL, nonconforming, present `L=0` GDT code only
- Push return `CS` + `IP`/`EIP`, then load CS cache (mirrors far JMP checks)

## Out of scope

- Call gates, task gates, privilege-changing CALL
- LDT targets, conforming outer transfers
- `VERR`/`VERW`

## Spec

Intel SDM Vol. 2 CALL; Vol. 3 §§5.8.1, 6.13.
