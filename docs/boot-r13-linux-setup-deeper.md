# R13 Linux bzImage setup deepen

Milestone 2, round 13, boot-guest lane, slice 4 (Linux half).

## Scope

`classify_bzimage_setup_deeper` refines [`classify_bzimage_early`] next-steps:

| Condition | `BzImageNextStep` |
|---|---|
| version ≥ 2.02, not LOADED_HIGH, `cmd_line_ptr==0` | `need-cmdline-ptr` |
| version ≥ 2.10, LOADED_HIGH, `init_size==0` | `need-init-size` |
| else | prior early classify (`run-real-mode-setup` / `load-high-…`) |

## Honesty

- Still **not** a Linux shell or setup execution.
- Does not vendor a real bzImage.

## Spec

Linux `Documentation/x86/boot.rst` (`cmd_line_ptr` @ `0x228`, `init_size` @ `0x260`);
`docs/boot-r12-linux-bzimage-early.md`.
