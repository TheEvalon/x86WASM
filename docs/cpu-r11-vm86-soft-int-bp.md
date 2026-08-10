# Round 11 — Soft-int / `#BP` polish (INT3 / ICEBP / INTO)

## Shipped

* `INT3` (`CC`) from VM86 delivers vector 3 (`#BP`) through a 386 IDT gate
  with the privilege-changing **9-dword** VM86→CPL0 frame (Figure 20-2), even
  when `IOPL < 3` (INT3 is not IOPL-sensitive; Table 20-1 / Vol. 2).
* `INTO` (`CE`) with `OF=1` likewise delivers `#OF` (vector 4) via that frame
  at `IOPL=0`; with `OF=0` it falls through and stays in VM86.
* `ICEBP` / `INT1` (`F1`) remains a **host decode miss**
  (`ExecError::Decode(UnsupportedOpcode(0xF1))`) — not silent `#DB` delivery.

## Unsupported (explicit)

* `ICEBP`/`INT1` (`F1`) semantics / `#DB` (vector 1)
* VME soft-int redirect bitmap
* 16-bit IDT gates from VM86
* Task gates as INT targets from VM86

## Spec

Intel SDM Vol. 2 "INT n/INTO/INT3/INT1"; Vol. 3 §§6.4, 20.2.2, Figure 20-2;
Table 20-1.

## Files

* `crates/x86-interpreter/src/lib.rs` — `0xCC` / `0xCE` (unchanged semantics;
  ICEBP still absent from the primary map)
* `crates/x86-interpreter/tests/cpu_r11_vm86_soft_int_bp.rs`
* `docs/cpu-r11-vm86-soft-int-bp.md`
