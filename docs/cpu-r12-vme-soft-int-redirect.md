# Round 12 — VME soft-int redirect bitmap stub

## Shipped

* When `CR4.VME=1` and `EFLAGS.VM=1`, software `INT n` (`0xCD`) consults the
  32-byte interrupt-redirection bitmap in the current 32-bit TSS (ends at the
  I/O map base at TSS offset `66h`).
* **Bit clear**: redirect through the 8086 IVT at **linear address 0**; push
  FLAGS/CS/IP on the VM86 stack; clear IF/TF; remain in VM86 (Table 20-2
  methods 5/6 class).
* **Bit set** (or map absent / incomplete): existing IOPL / protected-mode IDT
  path (`#GP(0)` when `IOPL < 3`).
* Missing map (`I/O base < 32` or not fully inside `TR.limit`): treat all bits
  as set (no silent IVT invent).
* Applies only to `INT n`. `INT3` / `INTO` are not redirected.

## Unsupported (explicit)

* Method-6 FLAGS image rewrite (`IOPL` forced to 3, `VIF→IF` on the pushed
  word) — stub pushes the live FLAGS image.
* VIF-based `CLI`/`STI`, VIP∧VIF `#GP`
* `CPUID.VME` (remains clear)
* Hardware IRQ / exception redirection (bitmap is soft-int only)

## Spec

Intel SDM Vol. 3 §§20.2–20.3 Table 20-2 / Figure 20-5; Vol. 3 §7.2.1; Vol. 2 INT n.

## Files

* `crates/x86-interpreter/src/lib.rs` — `vme_soft_int_redirect_bit_set`,
  `vm86_vme_redirect_soft_int`, `vm86_software_int_n`
* `crates/x86-interpreter/tests/cpu_r12_vme_soft_int_redirect.rs`
* `docs/cpu-r12-vme-soft-int-redirect.md`
