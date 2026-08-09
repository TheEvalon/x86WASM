# 32-bit paging translation engine

Milestone 2, round 3. Lives in `crates/x86-mmu/src/paging/`.

## Status: standalone, not wired

**No guest can use this.** The engine translates linear addresses to physical
addresses on demand, but nothing calls it. The interpreter's memory path still
treats a linear address as a physical address, `MOV CR3` / `MOV CR4` do not
exist, `CR0.PG` is not honored, and `#PF` is never delivered. Wiring it into the
interpreter is round-4 work; `docs/mmu-r3-integration-surface.md` states exactly
what that requires.

Consequently this document describes tested behavior of a library, not observed
behavior of a machine. Its only verification is its own unit and integration
tests: there is no firmware in the tree that enables paging.

## Authority

Intel SDM Vol. 3 Chapter 4 "Paging" is the only source used:

| Rule | Section |
|---|---|
| Paging-mode selection from `CR0.PG` / `CR4.PAE` | §4.1.1 |
| `CR0.WP`, `CR4.PSE`, `CR4.PGE` as paging-mode modifiers | §4.1.3 |
| Feature enumeration (PAT, PSE-36, MAXPHYADDR) | §4.1.4 |
| 32-bit walk, entry selection, final physical address | §4.3 |
| `CR3` fields | Table 4-3 |
| PDE mapping a 4-MiB page | Table 4-4 |
| PDE referencing a page table | Table 4-5 |
| PTE mapping a 4-KiB page | Table 4-6 |
| Access rights | §4.6.1 |
| Page-fault error code | §4.7 |
| Accessed and dirty flags | §4.8 |
| TLBs, global pages, invalidation | §4.10.2, §4.10.2.4, §4.10.4 |

No implementation from another emulator was read or copied.

## Shape of the API

```rust
// Caller-supplied physical memory. No dependency on machine-pc.
trait PageTableMemory {
    fn read_entry_u32(&mut self, phys_addr: u64) -> u32;
    fn write_entry_u32(&mut self, phys_addr: u64, value: u32);
}

// Control-register state plus the processor-model profile.
PagingContext { cr0, cr3, cr4, profile: PagingProfile }

// A pure structural walk: no permission check, no accessed/dirty write.
walk(&PagingContext, &mut impl PageTableMemory, linear: u32)
    -> Result<Walk, WalkError>

// Walk plus the §4.6 access-rights check. What an interpreter calls.
translate(&PagingContext, &mut impl PageTableMemory, linear: u32, Access)
    -> Result<Translation, TranslateError>
```

`TranslateError` separates the two failure kinds that must never be confused:

* `Fault(PageFault)` — architectural. The interpreter delivers `#PF` with the
  linear address in `CR2`.
* `Unsupported(UnsupportedPaging)` — a mode this engine does not model
  (`CR0.PG = 0`, `CR4.PAE = 1`). Reported rather than guessed, and never
  deliverable as a guest-visible exception.

Physical addresses are `u64` throughout, because 32-bit paging produces up to 40
of them when the PSE-36 mechanism is in use (§4.3).

## Walk rules implemented

* Page directory at `CR3` bits 31:12 (Table 4-3). `CR3` bits 2:0 and 11:5 are
  ignored, and bits 63:32 are ignored with 32-bit paging even on an Intel 64
  processor.
* `CR3.PWT` (bit 3) and `CR3.PCD` (bit 4) are **stored but inert**. They select
  only the memory type used to access the page directory (§4.9), and this
  engine models no caches and no memory types. `PagingContext::cr3_write_through`
  and `cr3_cache_disable` read them back so a future memory-typing slice has a
  seam, and nothing else consults them. The same is true of the PWT/PCD bits in
  each PDE and PTE.
* PDE index from linear bits 31:22, PTE index from linear bits 21:12, page
  offset from linear bits 11:0 (§4.3).
* `P = 0` in any entry used → no translation (§4.3, §4.7).
* Reserved bits, exactly as §4.3 states them: **with 32-bit paging there are
  reserved bits only if `CR4.PSE = 1`**, and only in an entry whose `P` flag is
  1.
  * PTE bit 7 is reserved when the PAT is not supported.
  * A PDE with `P = PS = 1` reserves bit 12 when the PAT is not supported, and
    reserves bits 21:13 when PSE-36 is not supported (bits 21:(M–19) when it
    is, where M is the minimum of 40 and MAXPHYADDR).
  * A PDE that references a page table has no reserved bits (Table 4-5).
  * Reserved bits are not checked in an entry whose `P` flag is 0 (§4.7, RSVD
    note), so such an entry reports a not-present fault.
* `PS` (PDE bit 7) is ignored when `CR4.PSE = 0`, so the entry references a page
  table (Table 4-5).
* With `CR4.PSE = 1` and `PS = 1` the PDE maps a 4-MiB page and no page table is
  read: the frame is PDE bits 31:22 (plus PDE bits 20:13 as physical bits 39:32
  under PSE-36) and the offset is linear bits 21:0 (§4.3, Figure 4-3).

## Accessed and dirty flags (§4.8)

`A` is set in every entry the translation used; `D` is set only in the entry
that identifies the final physical address — the PTE, or the PDE of a 4-MiB
page. Bit 6 of a PDE that references a page table is ignored (Table 4-5) and is
never written. Both flags are sticky, so an entry that already has them is not
rewritten at all, and an entry that needs both gets them in a single write.

### Ordering against faults — the one model choice

`walk` is a pure read. `translate` writes flags **only after** the walk
succeeded and the access-rights check passed, so **a faulting access leaves the
paging structures byte-for-byte unchanged**. Two places where the SDM's own
wording is looser than that:

* An entry with `P = 0` or a reserved bit "is used neither to reference another
  paging-structure entry nor to map a page" (§4.3), so its own `A` is certainly
  untouched — but a higher-level entry *was* read, and a literal reading of §4.8
  ("whenever the processor uses a paging-structure entry as part of
  linear-address translation") would set that entry's `A`. This engine does not.
* §4.6 checks rights after the translation has been produced, so a literal
  reading would also set `A` on a protection violation. This engine does not.

The tighter rule this follows is §4.10.2.3: "the processor does not cache a
translation for a page number unless the accessed flag is 1 in each of the
paging-structure entries used during translation; before caching a translation,
the processor sets any of these accessed flags that is not already 1." That ties
`A` to a translation that actually completes.

Stated plainly because it is a choice, not a derivation: the difference is
unobservable to conforming software, since these flags are sticky hints and
§4.10.4.2 warns software that it "cannot interpret the bit being clear as an
indication that such an access has not occurred". Erring toward fewer updates
cannot invent an access that never happened; erring the other way could tell a
guest a page was touched when the instruction faulted before touching it.

## Access rights (§4.6.1)

Rights combine over every entry the translation used: the address is a
user-mode address only if `U/S = 1` in **both** the PDE and the PTE, and the
translation is writable only if `R/W = 1` in both. §4.10.2.2 describes a TLB
entry as holding exactly those two logical-ANDs, which is why `Translation`
reports them.

What 32-bit paging without SMEP, SMAP, protection keys or execute-disable
reduces to:

| Access | Supervisor (CPL < 3) | User (CPL = 3) |
|---|---|---|
| Data read | always permitted | requires a user-mode address |
| Instruction fetch | always permitted | requires a user-mode address |
| Data write | `CR0.WP = 0`: permitted. `CR0.WP = 1`: requires combined `R/W = 1` | requires a user-mode address **and** combined `R/W = 1` |

The supervisor write row is the one worth stating twice: with `CR0.WP = 1` a
supervisor write is denied to a read-only page whether that page is a
supervisor-mode or a **user-mode** address. That is the case `CR0.WP` exists
for, and §4.6.1 spells it out under both "Data writes to supervisor-mode
addresses" and "Data writes to user-mode addresses".

Supervisor reads are unconditionally permitted here only because SMAP is not
modeled. §4.6.1 makes the implicit-versus-explicit supervisor-access
distinction solely to describe SMAP with `EFLAGS.AC`, so `AccessMode` does not
carry it; a future SMAP slice must add it.

## Page-fault error code (§4.7)

`PageFault::error_code()` composes the code from the fault reason and the
access, never from the access rights:

| Bit | Name | Set when |
|---|---|---|
| 0 | P | the fault was **not** a not-present fault — that is, for a protection violation or a reserved-bit violation |
| 1 | W/R | the causing access was a write |
| 2 | U/S | a user-mode access caused the fault |
| 3 | RSVD | a reserved bit was set in one of the entries used |
| 4 | I/D | never; see below |
| 5 | PK | never — no protection keys |
| 15 | SGX | never — no SGX |

Two traps this gets right on purpose:

* **RSVD implies P.** Reserved bits are not checked in an entry whose `P` flag
  is 0, so bit 3 can be set only if bit 0 is (§4.7). The reserved-bit checks in
  the walker are therefore guarded on `present()`.
* **I/D is not "this was a fetch".** §4.7 sets bit 4 only if the access was an
  instruction fetch *and* either `CR4.SMEP = 1` or (`CR4.PAE = 1` and
  `IA32_EFER.NXE = 1`). With 32-bit paging and no SMEP, none of those hold, so a
  faulting instruction fetch here produces a code with bit 4 **clear**. Setting
  it would be the more "obvious" behavior and would be wrong.

`PageFault::cr2()` returns the faulting linear address verbatim, offset
included.

## Processor-model profile

`PagingProfile` carries the three §4.1.4 facts that decide reserved bits and the
width of a 4-MiB page frame: PAT support, PSE-36 support, and MAXPHYADDR. The
default is the profile this emulator's CPUID actually reports — no PAT, no
PSE-36, 32 physical address bits — so the default engine behavior matches what a
guest would infer from `CPUID.01H:EDX`.

PSE-36 is implemented because §4.3 and Table 4-4 state it completely: physical
bits 39:32 come from PDE bits 20:13, and bits 21:(M–19) are reserved with M the
minimum of 40 and MAXPHYADDR. It is **off by default**, so the default engine
produces 32-bit physical addresses and treats PDE bits 21:13 as reserved. Turn
it on with `PagingProfile::with_pse36(maxphyaddr)`; physical addresses are `u64`
so the wider frames need no API change.

**This matters for round 4.** `CPUID` in this tree reports exactly one feature
bit (`MSR`); `PSE` (`EDX[3]`), `PGE` (`EDX[13]`), `PAT` (`EDX[16]`) and `PSE-36`
(`EDX[17]`) are all clear. Per §4.1.4 a guest may set `CR4.PSE` or `CR4.PGE`
only when the corresponding CPUID bit is set. Whoever wires `MOV CR4` must
either advertise those bits (they are now implemented, so advertising them would
be truthful) or reject writes that set them. Silently honoring an unadvertised
`CR4` bit would break the truthful-CPUID rule from the other direction.

## TLB (§4.10.2, §4.10.4)

**An architectural-correctness model, not a performance cache.** The walker is
already a couple of array reads, so nothing here is about speed. A TLB is
modeled because it is architecturally visible: software that edits a
paging-structure entry without invalidating must observe the stale translation,
the way it would on silicon. A guest that forgets an `INVLPG` should misbehave
here too, not work by accident.

The implementation is therefore deliberately naive — an unbounded map keyed by
4-KiB page number, no capacity, no replacement policy, no associativity. §4.10.2.2
allows exactly that ("Processors need not implement any TLBs. Processors that do
implement TLBs may invalidate any TLB entry at any time"), and a size-limited
model would only add nondeterminism to tests.

An entry holds what §4.10.2.2 says it holds: the page frame, the logical-AND of
the R/W flags, the logical-AND of the U/S flags, and the dirty flag of the entry
that maps the page — plus that entry's physical address, so a write through a
cached translation can still set `D` in memory. `CR0.WP` is deliberately *not*
cached: it is a paging-mode modifier applied per access (§4.1.3), which is why
changing it needs no invalidation.

An entry is created only from a translation that completed, which by then has
`P = 1`, no reserved bits, and `A = 1` in every entry used — the §4.10.2.3
precondition.

| Event | Effect | Section |
|---|---|---|
| `INVLPG <linear>` | invalidate entries for that page number, **including global ones**; for a 4-MiB page, every 4-KiB entry cached for it | §4.10.4.1 |
| Page fault | same invalidation, for the faulting address | §4.10.4.1 |
| `MOV to CR3` | invalidate everything **except global entries** | §4.10.4.1 |
| `MOV to CR0` clearing `PG` | invalidate everything, global included | §4.10.4.1 |
| `MOV to CR4` changing `PGE` | invalidate everything, global included | §4.10.4.1 |
| `MOV to CR4` changing `PAE` | invalidate everything | §4.10.4.1 |
| `MOV to CR4` changing `PSE` | invalidate everything — **model choice** | §4.10.4.1 |
| `MOV to CR0` changing `WP` | nothing | §4.1.3 |

Two of those deserve the emphasis:

* **Global pages are not exempt from a `CR3` load by having `G = 1`.** An entry
  is global only if it was cached from a final entry with `G = 1` *while*
  `CR4.PGE = 1` (§4.10.2.4). With `CR4.PGE = 0` no entry is global, so a `CR3`
  load is a full flush even for pages whose `G` bit is set. `CR4.PGE` is fully
  implemented here, exemption included, rather than left as an unsupported bit.
* **The `CR4.PSE` flush is a choice.** §4.10.4.1 says `MOV to CR4` *may*
  invalidate when `CR4.PSE` changes; it does not require it. This model always
  flushes, because the change reinterprets every `PS` bit, and §4.10.2.3 warns
  that leaving translations of both page sizes cached makes which one is used
  implementation-specific.

`Mmu` exposes both an explicit interface (`invlpg`, `on_mov_to_cr0`,
`on_mov_to_cr3`, `on_mov_to_cr4`) and a polled one
(`sync_control_registers(&ctx)`) that applies whatever invalidation the current
control-register values imply since it last looked. The polled form exists
because an integration changes `CR3` in more places than `MOV to CR3` — a task
switch, a reset — and missing one of those is the easiest way to end up with a
silently stale TLB.

## Deliberately out of scope

Named so they are not mistaken for oversights: PAE paging (§4.4), 4-level and
5-level paging (§4.5), long mode, `CR4.SMEP` / `CR4.SMAP` / `EFLAGS.AC`
(§4.6.1), `CR4.PKE` protection keys (§4.6.2), `IA32_EFER.NXE` execute-disable,
PCIDs and `INVPCID` (§4.10.1), the paging-structure caches (§4.10.3), memory
typing from PWT/PCD/PAT and the MTRRs (§4.9), SGX-induced page faults (§4.7
bit 15), multiple logical processors, speculative or prefetch-driven caching of
translations that never occur (§4.10.2.3), and shadow or nested paging.

Also absent for a different reason — they belong to the caller, not to the
engine: splitting a memory access that straddles a page boundary into two
translations, the `#PF` delivery mechanism itself, and the alignment and
segment-limit checks that precede paging.
