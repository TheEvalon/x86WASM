# SeaBIOS POST stop at `F000:C897` — diagnosis (post-c897)

## Ownership

Worktree `D:/x86WASM-wt-post-c897`, branch `slice/post-c897-fix` (base
`merge/m2-r11-parallel-16` @ `01050d2`).

**Owns:** `devices::Cf9Reset` (`0xCF9`), `MachineBus` CF9 ordinary-I/O decode,
`Machine::service_8042_pulse_reset` CF9 OR, POST diagnosis docs. Does **not**
own `x86-interpreter`, VGA, IDE, or PCI CONFIG_ADDRESS dword latching at `0xCF8`.

## Symptom (R11 tip, 2M-step probe)

```text
post-probe: steps=1281021 stop=step-budget-exhausted
  stop-pc        cs:ip=F000:C897 … bytes=[FA FC 66 C3 …]
  halt-idle      idle-pct≈35%
  spin-pc        hot: F000:9AE9…9AF0 (memset byte clear loop)
```

## Evidence

1. **ROM bytes at `F000:C895`:** `FB F4 FA FC 66 C3` = `sti; hlt; cli; cld; ret`
   — SeaBIOS `wait_irq` (`src/stacks.c` rel-1.16.3). Stop PC `C897` is the
   `cli` after HLT when the step budget ends mid-yield — **not** a decode gap.

2. **Hot spin PCs `F000:9AE9`:** `test edx; jz; dec; mov [eax+edx],0; jmp`
   — SeaBIOS `memset` (`src/string.c`) clearing a buffer one byte at a time.

3. **Trailing post-trace (2M):** RTC seconds (`0x70`/`0x71` index 0) + PIC EOI
   (`OUT 0x20,0x20`) + port `0x92` — `clock_poll_irq` / `check_irqs` around yields.

4. **20M-step probe (before CF9 fix):** stops at
   `cs:ip=0008:5A26` with `opcode_bytes=[CC …]` and
   `protected-mode delivery for vector 3 failed: IDT limit excludes the vector gate`.

5. **ROM at `0xF5A26`:** `mov edx,0xCF9` / `out 0x02` / `out 0x06` / `int3` —
   SeaBIOS `qemu_reboot` (`src/fw/shadow.c`):
   ```c
   outb(0x02, PORT_PCI_REBOOT);
   outb(0x06, PORT_PCI_REBOOT);
   asm volatile("int3");
   ```
   Call chain: `boot_fail` → `reset` → `entry_post` (HaveRunPost≠0) →
   `entry_resume` → `tryReboot` → `qemu_reboot` (`src/boot.c`, `src/resume.c`,
   `src/romlayout.S`).

6. **CF9 was silently dropped:** PCI owns `0xCF8`–`0xCFF` for Mechanism #1, but
   only a **DWORD at `0xCF8`** is CONFIG_ADDRESS (PCI 3.0 §3.2.2.3.2). Byte writes
   to `0xCF9` were no-ops → INT3 fallthrough → `#BP` delivery failed under the
   short SeaBIOS IDT.

## Causal gap (not C897)

| Layer | Finding |
|---|---|
| `F000:C897` | Sampling artifact of `wait_irq` during late POST yields |
| Real terminal (larger budget) | POST **reaches** `prepareboot` / INT 19h; **no bootable device** → `boot_fail` → CF9 reboot stub |
| Machine gap | ICH Reset Control Register at `0xCF9` unimplemented |

## Fix (this slice)

`devices::Cf9Reset` claims **non-DWORD** I/O at `0xCF9`. Writing bit2 (`RST_CPU`)
latches the shared system-reset path (`Machine::service_8042_pulse_reset`), same
as 8042 `0xFE` / port `0x92` bit0. DWORD at `0xCF8` CONFIG_ADDRESS unchanged.

## Remeasure notes

- Default 2M-step probe may still sample `F000:C897` during yields; use
  `--steps 20000000` (or higher) to reach boot_fail / CF9.
- With CF9 live and **no boot media**, SeaBIOS reboot-loops (expected on QEMU
  too). Measured after the fix: 20M steps → stop `F000:9842`, unclaimed
  `0x3E9`/`0x2E9` count=8 (≈8 POST cycles). See `docs/post-c897-remeasure.md`.
  That is **POST complete + boot_fail**, not a CF9 hang.
- Next slice for M2 exit: attach bootable HDD/CD or classify
  `cf9.reset_pulse_count() ≥ 1` after a long probe as “POST completed”.

## Spec / sources

- Intel ICH Reset Control Register at `CF9h` (SYS_RST / RST_CPU)
- PCI 3.0 §3.2.2.3.2 (CONFIG_ADDRESS dword-only)
- SeaBIOS rel-1.16.3 `src/fw/shadow.c` (`qemu_reboot`), `src/resume.c`
  (`tryReboot`), `src/boot.c` (`boot_fail`), `src/romlayout.S` (`entry_post`)
- Pinned image: `firmware/manifests/seabios.json` (`rel-1.16.3`)
