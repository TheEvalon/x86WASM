# POST remeasure after CF9 — post-c897

## Method

```text
emulator-cli --bios firmware/seabios/bios.bin --steps N --post-probe --post-spin 4096
```

Branch `slice/post-c897-fix` after `devices::Cf9Reset` + MachineBus wire.

## Results (2026-08-10)

### 2M steps (legacy budget)

```text
post-probe: steps=1281021 stop=step-budget-exhausted
  stop-pc        cs:ip=F000:C897 …
```

Unchanged vs R11 — budget still ends inside late-POST `wait_irq` yields.
**Not a regression; not the causal hang.**

### 20M steps (past C897)

```text
post-probe: steps=11753070 stop=step-budget-exhausted
  stop-pc        cs:ip=F000:9842 cs.d=0 eip=0x00009842
                 bytes=[66 3D 80 D5 00 00 75 0F]  ; cmp eax,0xD580 / jnz
  halt-idle      idle-pct≈41%
  spin-pc        hot: F000:9AE9…9AF0 (memset)
  unclaimed-port 0x3E9/0x2E9 count=8 each  ; ~8 POST cycles after CF9 reboot loop
```

| Metric | Before CF9 (20M) | After CF9 (20M) |
|---|---|---|
| Stop | INT3 @ `0008:5A26` (IDT/#BP fail) | `F000:9842` (real-mode yield/poll) |
| Past `C897`? | yes (then INT3) | **yes** (reboot loop) |
| COM3/4 probes | 1 | **8** (POST re-entry) |

## Interpretation

SeaBIOS POST completes far enough to run INT 19h, finds no boot media,
`boot_fail` → `qemu_reboot` → CF9 now resets the machine. Without a disk/CD
the guest reboot-loops; that is expected. M2 “POST complete” is effectively
reached; remaining exit is bootable media / FreeDOS measure.

See `docs/post-c897-cf9-diagnosis.md`.
