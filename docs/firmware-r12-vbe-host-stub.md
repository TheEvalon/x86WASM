# Firmware note: host VBE AX=4F00h stub (R12)

Milestone 2, Round 12, display-fw lane.

SeaBIOS / SeaVGABIOS binaries are **not** required for the host INT 10h
AX=4F00h controller-info stub. Harnesses call `Machine::service_int10` after
loading AX/ES/DI.

Until a validated SeaVGABIOS option ROM is mapped and its entry point is
invoked, treat this as host bring-up only. Capabilities remain clear and
PhysBasePtr stays zero — do not document a guest LFB aperture.

See `docs/vga-r12-vbe-4f00-info.md` and `docs/firmware-r9-seavgabios-linux-smoke.md`.
