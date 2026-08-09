# VGA plane (map) memory model

Scope note for the `devices::VgaText` VGA model. This file records the
specification text each behavior is derived from and the places where this
emulator makes an explicit model choice because the specification is silent.

## Approved sources used here

- IBM PS/2 Hardware Interface Technical Reference — Video Subsystems
  (form 42G2193, Sep 1992), chapter 2 "VGA Function":
  - Figure 2-29 Map Mask Register, index hex 02
  - Figure 2-33 Memory Mode Register, index hex 04
  - Figure 2-34 Map Selection, Chain 4
  - Figures 2-66 … 2-77 Graphics Controller registers, including
    Figure 2-73 Write Mode Definitions and Figure 2-75 Video Memory Assignments
- OSDev Wiki "VGA Hardware" — "Addressing Logic" (Chain 4 / Odd-Even offset
  forms observed on real hardware and emulators) and "Read/Write logic"
  (latch behavior, per-step write-mode algorithms)
- FreeVGA Sequencer / Graphics Registers (already listed in `docs/sources.md`)

`docs/sources.md` should gain the IBM PS/2 Video Subsystems reference; that file
is owned by the integration coordinator for this round.

## Address decode (Sequencer Memory Mode, index `0x04`)

| Programming | Maps selected | Map offset |
|---|---|---|
| Chain 4 (bit3 set) | `1 << (A[1:0])` — Figure 2-34 | host offset with A1:A0 cleared |
| Odd/Even (bit2 clear) | even → maps 0+2, odd → maps 1+3 — Figure 2-33 | host offset with A0 cleared |
| Planar (bit2 set, bit3 clear) | all four maps | host offset unchanged |

The host offset is measured from the base of the CPU display window.

The selected maps are then ANDed with the Map Mask register (Figure 2-29;
OSDev VGA Hardware, Write Mode 0: "The Memory Plane Write Enable field is ANDed
with the input from the address logic"). IBM notes that all maps should be
enabled while Chain 4 is selected; this model still applies the Map Mask, which
matches the QEMU / ATI / NVidia column of the OSDev comparison table.

Extended Memory (bit1) selects 64 KB or 256 KB of video memory. IBM documents
the size, not what happens to a host access beyond the 64 KB configuration, so
this model wraps the per-map offset within the enabled region
(16 KiB per map when clear, 64 KiB when set).

## Explicitly not modeled

- QEMU's alternative Chain-4 offset (`addr >> 2`); this model follows the
  officially documented / hardware-observed form.
- Chain-4 effects on the *display* fetch path (Bochs models one; real hardware
  per OSDev does not). There is no renderer in this model at all.
- Doubleword / word CRTC addressing (`Count by Two`, byte/word mode), which is
  what compensates for the odd/even offset form on the display side.
