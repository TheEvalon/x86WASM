# R9 Linux serial-path measure harness

Milestone 2, round 9, boot-guest lane, slice 4.

## Scope

Measure a path **toward** a 32-bit Linux serial console without claiming boot:

| Entry | Role |
|---|---|
| `synthetic_linux_serial_stub_disk` | MBR prints `LX` to COM1 then `HLT` |
| `Machine::measure_linux_serial_path` | Attach stub if needed; measure + gap list |
| CLI `--guest-linux-serial-measure` | Run without a host bzImage |

COM1 capture proves the harness can observe serial output from a guest stub.
It does **not** mean Linux reached userspace or earlyprintk.

## Documented gaps (not M2 exit)

- No bzImage / vmlinux fixture vendored or loaded
- No Linux boot protocol (real-mode setup header, protected-mode jump)
- No earlyprintk / 8250 console path through a real kernel
- Guest INT 13h still needs SeaBIOS for disked bootloaders
- Protected-mode / paging / CPUID gaps may still block real kernels

## Honesty

Reports always say **NOT an OS boot / NOT Milestone 2 exit**. Supplying a
host `--ide-image` only measures whatever that image does at `0x7C00` under
the same honesty rules.

## Spec

IBM PC BIOS INT 19h handoff; 16550 COM1 capture via existing POST probe;
Linux boot protocol is **out of scope** for this slice (gap only).
