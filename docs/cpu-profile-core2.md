# CPU profile — Core 2 era (target)

Long-term guest-visible CPUID presentation targets a Core 2 Conroe/Penryn–class feature set suitable for Windows 10 x64. See `plan.md`.

## Milestone 1 policy

Until features exist in the interpreter:

- Do **not** set CPUID feature bits for unimplemented ISA or devices.
- Report a conservative vendor/brand string and family/model only as needed for bring-up.
- Prefer `#UD` / explicit unsupported paths over silent wrong behavior.

## Reset (lab defaults)

Aligned with Intel SDM processor reset values used for real-mode bring-up:

| Field | Value |
|---|---|
| `RIP` | `0x0000_FFF0` |
| `RFLAGS` | `0x0000_0002` (bit 1 set) |
| `CS.selector` | `0xF000` |
| `CS.base` | `0xFFFF_0000` |
| `CS.limit` | `0xFFFF` |
| Other segment selectors | `0` |
| Other segment bases | `0` |
| Other segment limits | `0xFFFF` |
| `CR0` | `0x6000_0010` (ET set; PE clear) |

GPRs are `u64` storage; only low bits are architecturally meaningful in real mode.

## Spec refs

- Intel SDM Vol. 3: processor initialization and reset values
- Intel SDM Vol. 2: CPUID
