# Paging wired into the interpreter (round-4)

Milestone 2, round 4. This is the other half of
`docs/mmu-r3-integration-surface.md`: the round-3 engine in
`crates/x86-mmu/src/paging/` is now reachable from executing guest code.

## Status: a guest can enable paging and run under it

`CR0.PG = 1` translates every data access, every instruction fetch and every
descriptor-table read; a translation failure is delivered as `#PF` (vector 14)
through the existing 386 interrupt gate with the linear address in `CR2` and
the §4.7 doubleword error code on the stack; the instruction then re-executes.
A handler that repairs the mapping and executes `IRETD` resumes the faulting
instruction. Tests in `crates/x86-interpreter/tests/cpu_r4_*.rs` do exactly
that with page tables built in guest memory.

What still prevents a real 32-bit OS from booting is not paging. See
[Not implemented](#not-implemented-and-why) below.

## Authority

| Rule | Section |
|---|---|
| `CR0.PG` requires `CR0.PE`; a linear address is physical when `PG = 0` | Vol. 3 §4.1.1 |
| `CR4.PSE`/`CR4.PGE` are settable only when CPUID advertises them | Vol. 3 §4.1.4 |
| `CR3` fields; bits 2:0 and 11:5 ignored, not reserved | Vol. 3 Table 4-3 |
| Access rights; implicit supervisor accesses to GDT/LDT/IDT/TSS | Vol. 3 §4.6.1 |
| `#PF` error code, `CR2` | Vol. 3 §4.7 |
| Accessed/dirty flags | Vol. 3 §4.8 |
| A translation may be cached only once its accessed flags are set | Vol. 3 §4.10.2.3 |
| `INVLPG`, `MOV to CR0/CR3/CR4` invalidation | Vol. 3 §4.10.4.1 |
| Segmentation produces the linear address paging then translates | Vol. 3 §3.3.1, §4.1.1 |
| Exception priority classes | Vol. 3 §6.9 Table 6-2 |
| `#PF` is a fault: `CS:EIP` names the faulting instruction | Vol. 3 §6.5 |
| Register state after a `REP` is suspended; the `CMPS`/`SCAS` `EFLAGS` rule | Vol. 2 "REP/REPE/REPZ/REPNE/REPNZ" |
| `moffs` offset width follows the address-size attribute | Vol. 2 "MOV" |
| `MOV to/from CRn` exceptions | Vol. 2 "MOV—Move to/from Control Registers" |

No implementation from another emulator was read or copied.

## Shape of the integration

Everything hangs off one type. `PagedBus` wraps the machine's `Bus` and is
what the interpreter actually talks to, **whether or not paging is on**, so the
translated and untranslated paths cannot drift apart:

```rust
let mut paged = PagedBus::new(machine_bus, &mut mmu, cpu);
step_paged(cpu, &mut paged)
```

* `CR0.PG = 0` forwards the linear address untouched, at its original width.
  That is the branch every pre-round-4 test still runs on, and it is why the
  engine returns `Unsupported(PagingDisabled)` instead of an identity map: the
  caller must decide explicitly.
* `CR0.PG = 1` translates. Data accesses use `AccessKind::Read`/`Write` at the
  current CPL; instruction bytes go through `Bus::fetch_u8` and use
  `InstructionFetch`; GDT and IDT bytes go through `Bus::read_system_u8` and
  use `AccessMode::Supervisor` regardless of CPL, per §4.6.1.

Four `Bus` methods carry the seams a machine integration may want, all with
defaults that preserve the pre-paging behavior: `on_mov_to_control_register`,
`invalidate_page`, `probe_write`, and `commit_string_iteration`.

### Fault ordering

`#GP`/`#SS` for a segment-limit violation necessarily precede `#PF`, because
the limit check happens while the linear address is being formed and paging
never sees an address that failed it (§3.3.1, §4.1.1). Table 6-2 puts a data
page fault and a general-protection fault in the same priority class and
declares ordering *within* a class implementation-dependent, so the pipeline
order is what decides, and it decides in favor of segmentation.

Fetch-side, a code-segment limit violation and a code page fault are likewise
one class (class 8), both ahead of decode faults such as `#UD`. That falls out
of the same ordering: `fetch_decode` limit-checks each byte before fetching it,
and it fetches before it decodes.

## Restartability

`#PF` is a fault, so the instruction re-executes and must have committed
nothing. Two mechanisms, deliberately different in kind:

**A checkpoint at the instruction boundary.** `PagedBus` clones the
architectural state before each instruction and restores it when a translation
fails. This is armed *only while `CR0.PG = 1`*, so the pre-paging execution
path is unchanged and pays nothing. It covers register, pointer and flag
commits uniformly rather than opcode by opcode: `PUSHA` faulting on its fifth
slot, `POPA` on its fifth, a read-modify-write whose store is refused after its
read has already set flags, `ENTER` faulting mid-display.

**A carve-out for `REP`.** The SDM does *not* say a suspended string operation
restarts from the beginning; it says the indices "point to the next string
elements to be operated on", `EIP` points at the string instruction, and `ECX`
"has the value it held following the last successful iteration". So the loop
publishes a new checkpoint after every completed iteration via
`Bus::commit_string_iteration`, and the faulting iteration alone is rolled
back. The checkpoint's `RFLAGS` is never advanced, which is the same section's
separate rule that a faulting `REPE`/`REPNE` `CMPS`/`SCAS` restores `EFLAGS`
to its pre-instruction value.

**What a checkpoint cannot undo is a memory write.** Two cases needed real
work rather than rollback:

* A single operand straddling a page boundary. `translate_span` probes both
  halves before translating either, so a second-half fault leaves neither the
  first half's bytes nor its accessed/dirty flags written. An access inside one
  page skips the probe and keeps its original width on the machine bus, which
  matters for MMIO.
* `INS`, whose port read precedes its store and cannot be replayed. The
  destination is probed first, so an unwritable destination costs no port
  cycle. `OUTS` needs nothing: its memory read already precedes its port write.

Everything else that writes more than one location before it can fault —
`PUSHA`, `ENTER`'s display, interrupt-gate frames — writes only to the stack
below the restored pointer, where the retry rewrites the same bytes. The gate
path additionally still restores the frame bytes it had already stored.

## `Mmu::probe`

New in this round, in `x86-mmu`: "would this access translate?", answered with
no side effect at all — no accessed flag, no dirty flag, nothing cached. It
consults the TLB, because the access it stands in for would and the two must
not disagree, and a miss walks read-only. §4.10.2.3 forbids caching a
translation before its accessed flags are set, which is exactly why a probe may
not populate the TLB. A probe that faults performs the §4.10.4.1 invalidation,
like any other page fault.

## Control registers

`CR2`, `CR3` and `CR4` are guest-readable and guest-writable; they used to
return `Unsupported`. `MOV to/from CRn` requires CPL 0, and `CR1`/`CR5`–`CR7`
remain `#UD`.

`CR4`'s reserved mask is *derived from the CPUID feature word*, so §4.1.4's
rule cannot drift from what CPUID says. Only `PSE` (bit 4) and `PGE` (bit 7)
are settable; every other bit — including `PAE` — raises `#GP(0)`. Refusing
`CR4.PAE` is what stops a guest selecting the paging mode the engine reports as
unsupported.

`CPUID.01H:EDX` therefore now reports `PSE` (bit 3), `MSR` (bit 5), `PGE`
(bit 13) and `CMOV` (bit 15), and the version signature moves from family 5 to
family 6 — the generation that introduced `PGE` and `CMOV` — so the signature
and the feature bits still agree. `PAE`, `PAT` and `PSE-36` stay clear; the
engine's default reserved-bit profile assumes exactly that.

`INVLPG` is no longer a real-mode NOP by special case. It forms and
limit-checks its operand address, requires CPL 0, and invalidates. With nothing
cached that is still a no-op, which is the architectural result rather than a
shortcut.

## The A/D-versus-fault deviation, revisited

`docs/mmu-r3-32bit-paging.md` records one model choice: a faulting access
leaves the paging structures byte-for-byte unchanged, which is tighter than a
literal reading of §4.8 and follows §4.10.2.3 instead.

**Wiring it to real instruction execution did not turn up a reason to revisit
it.** Two things were checked directly rather than assumed:

* `a_faulting_access_writes_no_paging_structure_byte` executes a store into a
  not-present page and asserts that the *higher-level* PDE the walk did read
  keeps its accessed flag clear.
* `a_faulting_split_write_sets_no_flag_on_the_reachable_half` executes a
  page-crossing store whose second half is absent and asserts the first half's
  PTE keeps both flags clear. That case only exists once a caller splits an
  access, and it is where the choice would have been easiest to violate by
  accident.

Nothing observed needs the looser rule: no guest code in the tree reads these
flags after a fault, and §4.10.4.2 forbids software from reading a clear flag
as "this did not happen". The choice stands, and it is now cheaper to keep
because `Mmu::probe` gives callers a first-class way to ask about a translation
without writing to it.

## Not implemented, and why

Reported explicitly rather than silently approximated:

* **PAE, 4-level and 5-level paging, long mode.** `CR4.PAE` is refused with
  `#GP(0)`; `ExecError::UnsupportedPaging` exists for a mode that reaches the
  engine anyway, and nothing in this build can produce one.
* **SMEP, SMAP, protection keys, execute-disable.** No `CR4` bit for them is
  settable, and the `#PF` error code's I/D bit is consequently always clear —
  §4.7 sets it only with SMEP or PAE+NXE.
* **`#DF` and triple fault.** A fault during exception delivery is reported to
  the host as `ProtectedModeExceptionDelivery`, not escalated. A `#PF` on the
  gate's own stack writes lands here.
* **Privilege-changing gates.** Delivery is same-CPL only, so a ring-3 `#PF`
  handler has to be a ring-3 code segment. Stack switching through a TSS is a
  separate slice, and until it exists `PagedBus` can sample CPL once per
  instruction.
* **A page-table walk that cannot reach physical memory** is
  `ExecError::PageTableFault`, not a fabricated not-present page.
* **Task-switch `CR3` loads.** `PagedBus::new` polls
  `Mmu::sync_control_registers` every instruction, so a `CR3` written by a path
  other than `MOV to CR3` is still noticed; there is no task switch yet to test
  it against.
* **A persistent TLB in the machine — done at integration.** `Machine` now holds
  an `x86_mmu::paging::Mmu` and calls `step_with_mmu` so the TLB persists across
  instructions; `Machine::reset` replaces it. Guests that forget `INVLPG` are no
  longer accidentally correct on every instruction.

## Fixed in passing

`moffs` offsets keyed on the presence of a `0x67` prefix rather than on the
effective address-size attribute, so under `CS.D = 1` every `moffs` reference
was truncated to its low word. Vol. 2 "MOV" states that "the address-size
attribute of the instruction determines the size of the offset". This was the
cause of SeaBIOS's write-to-ROM `#GP` storm at `0xFFFF6E06`: truncated 32-bit
POST writes landed at `CS.base + offset16` inside the ROM window. The POST
probe goes from 2,000,000 steps of `step-budget-exhausted` with 74,434 trace
events to 150,360 steps and a halt with 661, and the unmapped-MMIO write pages
disappear entirely.
