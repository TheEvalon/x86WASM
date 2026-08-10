# R10 Linux serial measure → first-failure classify

Milestone 2, round 10, boot-guest lane, slice 4.

## Scope

Same first-failure classification as FreeDOS (`GuestFirstFailureClass`, schema
v4) applied to `Machine::measure_linux_serial_path` /
CLI `--guest-linux-serial-measure`.

Tiny stub polish: synthetic MBR prints `LX\r\n` to COM1 (CRLF line ending)
before `HLT` so serial capture looks earlyprintk-shaped. This is **not** a
Linux boot-protocol or earlyprintk driver.

## Measured first failure (synthetic fixture)

With the in-tree Linux serial stub, the classified stop is **`synthetic-halt`**
after COM1 `LX\r\n`. That is **not** a Linux shell and **not** Milestone 2 exit.

## Documented boot-protocol gaps

- No bzImage / vmlinux fixture vendored or loaded
- No Linux boot protocol (real-mode setup header, protected-mode jump)
- No earlyprintk / 8250 console path through a real kernel
- Guest INT 13h still needs SeaBIOS for disked bootloaders
- Protected-mode / paging / CPUID gaps may still block real kernels

## Honesty

Reports always say **NOT an OS boot / NOT Milestone 2 exit**. Do not vendor a
real bzImage in this tree for the measure path.

## Spec

IBM PC BIOS INT 19h handoff; 16550 COM1 capture via existing POST probe;
Linux boot protocol remains a documented gap only
(`docs/boot-r9-linux-serial-measure.md`).
