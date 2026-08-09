# Two-byte `0F` opcode map (Milestone 2, round 2)

Scope: what the `x86-decode` / `x86-interpreter` two-byte map implements after
the round-2 CPU slices, and — just as importantly — what it does not.

Authority: Intel SDM Vol. 2 (instruction reference and Appendix A opcode maps),
Vol. 3 Ch. 5–6. No behavior here is taken from another emulator.

Before this round the map held only `0F 01` (Group 7), `0F 06` (`CLTS`),
`0F 20`/`0F 22` (`MOV` to/from `CR0`), and `0F AF` (`IMUL`).

## Shared rules

Every form added in this round obeys the round-1 size rules:

- The operand-size and address-size attributes come from the code segment's
  cached `D` bit; `0x66` and `0x67` select the *other* size, so they invert
  under `D=1` (SDM Vol. 1 §3.6 Table 3-4; Vol. 3 §3.4.5).
- Instruction-pointer commits go through the shared `set_current_ip`, which
  writes `IP` only under `D=0` and the full `EIP` under `D=1`.
- Byte register operands use the shared legacy `AL..BH` accessors, so `mod=11`
  encodings 4–7 address the high bytes.
- Memory operands go through the shared effective-address helpers, so cached
  segment limits, the `#GP`/`#SS` split, and 32-bit ModR/M + SIB addressing
  behave exactly as they do in the primary map.

## Slice 1 — `Jcc` near (`0F 80`–`0F 8F`) and `SETcc` (`0F 90`–`0F 9F`)

Spec: SDM Vol. 2 "Jcc—Jump if Condition Is Met", "SETcc—Set Byte on
Condition", Appendix B (condition-code encodings); Appendix A opcode map 2.

All sixteen condition codes are evaluated by one shared helper keyed on the low
nibble, so the short `70`+cc form, the near `0F 80`+cc form, and `0F 90`+cc
`SETcc` cannot disagree.

Supported:

- Near `Jcc` with a rel16 displacement (16-bit operand size), which clears
  `EIP[31:16]` on a taken branch, and with a rel32 displacement (32-bit operand
  size). Not-taken branches fall through to the sequential next `EIP`. Neither
  outcome writes flags.
- `SETcc r/m8` with a register destination (including `AH`/`CH`/`DH`/`BH`) and
  with a memory destination in both 16-bit and 32-bit addressing. Exactly one
  byte is written, `1` or `0`, and no flag is modified. `ModR/M.reg` is not
  used, so any value in that field decodes and executes.

Not supported:

- The `CS`-limit `#GP` for a near branch is still detected on the next
  instruction fetch rather than at branch time.
- A 32-bit operand-size near `Jcc` executed from a `D=0` code segment commits
  only `IP`, because `set_current_ip` owns that window rule for the whole
  interpreter.
- 64-bit mode (`rel32` sign-extended into `RIP`) and the branch hint prefixes
  `2E`/`3E`.

## Slice 2 — extension moves and segment loads

Spec: SDM Vol. 2 "MOVZX—Move with Zero-Extend", "MOVSX—Move with
Sign-Extension", "PUSH", "POP", "LDS/LES/LFS/LGS/LSS—Load Far Pointer";
Vol. 3 §§3.4.2–3.4.5, 5.4.1, 6.8.3, 6.15.

Supported:

- `MOVZX`/`MOVSX` (`0F B6`/`B7`/`BE`/`BF`) in all eight source/destination
  width combinations. The opcode fixes the source width and the operand-size
  attribute fixes the destination width, so a word source with a 16-bit
  operand size is an ordinary word move. A 16-bit destination leaves the upper
  half of the 32-bit register untouched. Byte sources reach memory and the
  legacy `AL..BH` registers. No flags are written.
- `PUSH FS`/`GS` and `POP FS`/`GS` (`0F A0`/`A1`/`A8`/`A9`). The stack slot is
  a word under a 16-bit operand size and a doubleword holding the zero-extended
  selector under a 32-bit one; the stack-pointer width itself still follows
  `SS.B`. In protected mode `POP FS`/`GS` validate the selector through the
  shared DS/ES data-descriptor path (null selectors allowed, clearing the
  cache) before either the stack pointer or the cache commits.
- `LSS`/`LFS`/`LGS` (`0F B2`/`B4`/`B5`) with `m16:16` and `m16:32` pointers.
  They reuse the same descriptor helpers as `LDS`/`LES`, so `SS` gets the
  stack-segment rules (null selector `#GP(0)`, writable ring-matched data
  required, `P=0` reported as `#SS`) and `FS`/`GS` get the DS/ES data rules.
  Nothing commits until the whole pointer is read and the descriptor validates.
  `LSS` arms the same maskable-interrupt shadow as `MOV SS`/`POP SS`;
  `LFS`/`LGS` do not. The register form (`mod=11`) is `#UD`.

Not supported:

- The primary-map `POP ES`/`SS`/`DS` (`07`/`17`/`1F`) and `PUSH ES`/`CS`/`SS`/`DS`
  (`06`/`0E`/`16`/`1E`) still use a 16-bit stack slot regardless of the
  operand-size attribute. That is pre-existing behavior outside this slice, but
  it is now inconsistent with `0F A0`/`A1`/`A8`/`A9` and should be a round-3
  item.
- `MOVSXD` and the REX.W 64-bit destinations.
- Loading `SS` from the LDT, or any privilege-level change.

## Slice 3 — bit and exchange instructions

Spec: SDM Vol. 2 "BT", "BTS", "BTR", "BTC", "BSF", "BSR", "BSWAP", "XADD",
"CMPXCHG"; Vol. 2 §3.1.1.9 (`Bit(BitBase, BitOffset)` notation); Vol. 2
Appendix A opcode map 2 and the Group 8 table; Vol. 3 §5.3.

Supported:

- `BT`/`BTS`/`BTR`/`BTC` in both the register bit-offset forms
  (`0F A3`/`AB`/`B3`/`BB`) and the Group 8 immediate forms (`0F BA /4`–`/7`),
  at both operand sizes. `/0`–`/3` of Group 8 are reserved and raise `#UD`.
  - A **register** bit base takes `BitOffset MOD OperandSize`, so a negative or
    oversized offset still selects a bit inside the register.
  - A **memory** bit base is the start of a bit string. The addressed bit is
    `BitOffset MOD 8` inside the byte at `BitBase + (BitOffset DIV 8)`, where
    the division is signed and rounds toward negative infinity and the modulo is
    non-negative. A register offset therefore reaches bits far above *and below*
    the nominal operand, and the segment-limit check applies to that displaced
    byte rather than to the bit base.
  - `CF` receives the original bit and commits only after the read-modify-write
    cannot fault.
- `BSF`/`BSR` (`0F BC`/`BD`) at both operand sizes, register and memory sources.
- `BSWAP` (`0F C8`–`0F CF`), `XADD` (`0F C0`/`C1`) with the full ADD flag
  results, and `CMPXCHG` (`0F B0`/`B1`) including the specified destination
  write-back on a mismatch and the CF/PF/AF/SF/OF results of the comparison.

Deterministic choices where the SDM says "undefined". The interpreter is the
semantic reference for a future JIT, so an indeterminate result would be
untestable; each of these is a legal instance of the undefined behavior:

- The `BT` family leaves `OF`, `SF`, `ZF`, `AF`, and `PF` unchanged.
- `BSF`/`BSR` leave the destination unchanged when the source is zero, and
  leave `CF`, `OF`, `SF`, `AF`, and `PF` unchanged.
- `BSWAP` with a 16-bit operand size performs the same full 32-bit byte
  reversal.

Not supported:

- The Group 8 immediate is reduced modulo the operand size. That is exact over
  the `0..OperandSize-1` range the SDM defines for the immediate form; larger
  immediates are outside the defined domain and are masked rather than
  extending the bit-string address.
- `LOCK` is decoded but has no atomicity effect (single-processor model), and
  `LOCK` with a register destination does not raise `#UD`.
- The REX.W 64-bit forms, `CMPXCHG8B`/`CMPXCHG16B`, and `TZCNT`/`LZCNT`.

## Slice 4 — system and identification

Spec: SDM Vol. 2 "CPUID", "RDMSR", "WRMSR", "UD2", "INVD", "WBINVD"; Vol. 3
§5.5; Vol. 4 (MSR listings).

### What CPUID reports, and why every bit is truthful

`AGENTS.md` forbids advertising an unimplemented feature. Under-reporting is
safe; over-reporting is not. The complete guest-visible output is:

| Leaf | EAX | EBX | ECX | EDX |
|---|---|---|---|---|
| `0` | `0000_0001` | `"x86W"` | `"Emu "` | `"ASM "` |
| `1` | `0000_0500` | `0000_0000` | `0000_0000` | `0000_0020` |
| `8000_0000` | `8000_0000` | `0` | `0` | `0` |
| anything else | same as leaf `1` | | | |

- **Leaf 0 EAX = 1** — the highest basic leaf with content. Leaf 2 (cache
  descriptors) and beyond are not implemented, so they are not claimed.
- **Vendor string `x86WASM Emu `** — deliberately neither `GenuineIntel` nor
  `AuthenticAMD`. Software commonly infers capabilities from a familiar vendor
  plus family/model rather than from feature bits, and a distinctive vendor
  removes that inference. `docs/cpu-profile-core2.md` already asks for a
  conservative vendor/brand string until the features exist.
- **Leaf 1 EAX = family 5, model 0, stepping 0** — family 5 is the generation
  that introduced `RDMSR`/`WRMSR`, which is the one feature bit reported, so the
  signature and the feature bits do not contradict each other. Nothing claims a
  specific shipping part.
- **Leaf 1 EBX = 0** — brand index, `CLFLUSH` line size, and maximum logical
  processors are only meaningful with feature bits that stay clear, and the
  single modeled processor's initial APIC ID is 0.
- **Leaf 1 EDX = bit 5 (`MSR`) only** — this bit means "the `RDMSR` and `WRMSR`
  instructions are supported", and they are: they decode, enforce CPL 0, and
  raise the architectural `#GP` for MSR addresses the processor does not
  implement. It does **not** claim that any particular MSR exists. Every other
  bit is clear because none of those features exist here: no `FPU` (no x87), no
  `TSC`, `DE`, `VME`, `PSE`, `PAE`, `PGE`, `PAT`, `MTRR`, `APIC`, `SEP`, `CX8`
  (no `CMPXCHG8B`), `CMOV`, `CLFSH`, `MMX`, `SSE`, `SSE2`, or `HTT`.
- **Leaf 1 ECX = 0** — no ECX-enumerated feature is implemented.
- **Leaf `0x8000_0000` EAX = `0x8000_0000`** — extended leaves are enumerable
  but none has content.
- **Out-of-range leaves** return the highest basic leaf, which is the documented
  behavior. This covers `0x4000_0000`, which firmware probes for a hypervisor
  signature: this emulator is not a hypervisor and presents no such signature.

### MSRs

`RDMSR`/`WRMSR` implement the full instruction mechanics — `ECX` selects the
MSR, `EDX:EAX` carries the 64-bit value, CPL 0 is required outside real-address
mode — but **no MSR is implemented**. Every address raises `#GP(0)`, which is
the architectural response for a reserved or unimplemented MSR address.

This is deliberate rather than a stub returning zero. The emulator models no
time-stamp counter, local APIC, MTRRs, `SYSENTER` state, or `EFER`, so there is
nothing it could report truthfully. Modeling MTRRs honestly is out of scope for
this slice; because CPUID leaf 1 leaves the `MTRR` bit clear, firmware that
checks CPUID before touching MTRR MSRs skips them entirely, which is what
SeaBIOS does.

### Other

- `UD2` (`0F 0B`) raises `#UD` in every mode — the architecturally guaranteed
  invalid opcode, distinct from a host decode gap.
- `INVD`/`WBINVD` (`0F 08`/`0F 09`) are architectural no-ops because no caches
  are modeled; only the CPL 0 requirement is observable. No external write-back
  cycle or cache-coherence effect is produced.

Not supported:

- Any implemented MSR, and therefore any `WRMSR` reserved-bit `#GP`.
- CPUID `ECX` sub-leaf selection (no leaf that uses it is implemented) and the
  extended leaves `0x8000_0001`+ (including the processor brand string).
- The `#UD` that a `LOCK` prefix should raise on `CPUID`, `RDMSR`, `WRMSR`,
  `INVD`, and `WBINVD`.
- Virtual-8086 mode, where `RDMSR`/`WRMSR` are not recognized.

## Remaining unimplemented `0F` opcodes

The map still has no entry for `0F 00` (Group 6 `LLDT`/`LTR`/`SLDT`/`STR`/
`VERR`/`VERW`), `0F 02`/`0F 03` (`LAR`/`LSL`), `0F 05`/`0F 07`/`0F 34`/`0F 35`
(`SYSCALL`/`SYSRET`/`SYSENTER`/`SYSEXIT`), `0F 0D`/`0F 18`–`0F 1F` (prefetch and
the multi-byte `NOP`), `0F 21`/`0F 23` (debug registers), `0F 31` (`RDTSC`),
`0F 33` (`RDPMC`), `0F 40`–`0F 4F` (`CMOVcc`), `0F A4`/`0F A5`/`0F AC`/`0F AD`
(`SHLD`/`SHRD`), `0F C7` (Group 9 `CMPXCHG8B`), `0F AE` (Group 15), `0F 09`-era
`0F 0F` 3DNow!, or any MMX/SSE/AVX opcode.
