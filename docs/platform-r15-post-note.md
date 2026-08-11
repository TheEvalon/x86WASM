# Platform-post note vs R14 POST-with-media (`F000:C897`) — Milestone 2 Round 15

## Context

R14 with INT19 HD: stop `F000:C897`, class `other-stop` (past `F000:9842`).

## This lane

Host stubs: INT15 AH=88/E801, CMOS↔BDA equipment, BDA `40:6C` from PIT IRQ0,
plus PIT wait_irq re-latch / PIC OCW3 honesty docs.

## Remeasure stance

No full 20M re-run claimed. Expected stop still `F000:C897`. **Not** POST complete.
