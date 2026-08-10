# Round 12 — INTO / #OF from VM86 under VME

## Shipped

* `INTO` with `OF=1` from VM86 delivers `#OF` (vector 4) through the protected-
  mode IDT with the 9-dword VM86→CPL0 frame.
* **Not** IOPL-sensitive (unlike `INT n`).
* **Not** governed by the VME interrupt-redirection bitmap (unlike `INT n`).
  Even with `CR4.VME=1` and redirect bit 4 clear, `INTO` uses the IDT while
  `INT 4` redirects to the IVT.
* Untaken `INTO` (`OF=0`) advances IP and stays in VM86.

## Unsupported (explicit)

* Method-6 VIF image details on redirected `INT n` (see redirect stub doc)
* VIF-based `CLI`/`STI`
* `CPUID.VME`

## Spec

Intel SDM Vol. 2 INT n/INTO/INT3; Vol. 3 §20.2.2 Table 20-2; Vol. 3 §6.15 (#OF).

## Files

* `crates/x86-interpreter/src/lib.rs` — `INTO` comments
* `crates/x86-interpreter/tests/cpu_r12_vm86_into_vme.rs`
* `docs/cpu-r12-vm86-into-vme.md`
