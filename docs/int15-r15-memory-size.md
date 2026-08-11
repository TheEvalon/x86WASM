# INT 15h AH=88h / AX=E801h memory-size stubs — Milestone 2 Round 15

## Why

FreeDOS and early Linux probes often call classic BIOS memory services before
(or instead of) e820. CMOS already publishes the same numbers; this slice
exposes them through INT 15h.

## Spec

RBIL INT 15h AH=88h / AX=E801h.

## Model

`Machine::service_int15_memory_size` reads live CMOS `17h`/`18h` / `34h`/`35h`
with the documented 15 MB / 64 KB-block caps.

## Honesty

Host stub only — not a guest IVT body; does not move SeaBIOS past `F000:C897`.

## Tests

`crates/machine-pc/src/int15_mem.rs`
