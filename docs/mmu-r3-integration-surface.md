# Wiring the paging engine to the interpreter (round-4 surface)

The 32-bit paging engine in `crates/x86-mmu/src/paging/` is complete and tested
but **connected to nothing**. This document is the contract a round-4 slice has
to satisfy. It states what to call, what state to thread in, how a fault becomes
a `#PF`, and where the work is expected to be harder than it looks.

Engine behavior itself is in `docs/mmu-r3-32bit-paging.md`.

## What the interpreter must call

One call per memory access, after segmentation has produced a linear address and
before touching physical memory:

```rust
let ctx = PagingContext::new(cpu.cr0, cpu.cr3, cpu.cr4);
let access = Access::from_cpl(AccessKind::Read, cpl);   // or Write / InstructionFetch
match mmu.translate(&ctx, &mut bus_page_memory, linear as u32, access) {
    Ok(t)  => /* use t.phys_addr */,
    Err(TranslateError::Fault(fault)) => /* deliver #PF, below */,
    Err(TranslateError::Unsupported(kind)) => /* report; never deliver */,
}
```

Three things about that call:

* **`CR0.PG = 0` returns `Unsupported(PagingDisabled)`, not the identity
  mapping.** The engine refuses to answer rather than quietly returning
  `linear as u64`, so a caller cannot forget the check. The interpreter's memory
  path should branch on `ctx.paging_enabled()` and keep its current
  linear-equals-physical behavior when paging is off. That also keeps every
  existing real-mode and protected-mode test on exactly the path it uses today.
* **`AccessKind` is not the same thing as "read or write".** An instruction
  fetch must be `InstructionFetch`, not `Read`. It makes no difference to the
  permission result with 32-bit paging (no SMEP, no NX), but it is what the
  error code's I/D bit and any future SMEP slice key on.
* **`Access::from_cpl` needs the *effective* privilege of the access.** Accesses
  the processor makes on software's behalf to the GDT, LDT, IDT and TSS are
  supervisor-mode accesses regardless of CPL (§4.6.1). Those paths must pass
  `AccessMode::Supervisor` explicitly rather than deriving it from CPL.

## Threading `PageTableMemory` in

```rust
trait PageTableMemory {
    fn read_entry_u32(&mut self, phys_addr: u64) -> u32;
    fn write_entry_u32(&mut self, phys_addr: u64, value: u32);
}
```

Implement it on whatever the interpreter already uses to reach physical memory —
`MachineBus`, or a thin wrapper over `PhysMem`. Requirements:

* Little-endian composition of four bytes.
* **The A20 gate applies**, because a page-table walk is an ordinary physical
  memory access. Route through the same masking `PhysMem` already does rather
  than around it.
* Reads take `&mut self` so a bus type fits without interior mutability.

The engine deliberately does not depend on `machine-pc`, exactly like the
`devices` bus-master transfers.

## Delivering the fault

`PageFault` carries everything the delivery path needs and raises nothing
itself:

| Field / method | Use |
|---|---|
| `fault.cr2()` | load into `CR2` **before** delivering |
| `fault.error_code()` | the doubleword error code pushed by the `#PF` gate |
| `fault.reason` | diagnostics only; the error code is the architectural part |

`#PF` is vector 14, a fault (`CS:EIP` points at the faulting instruction, which
re-executes), and it pushes an error code. `CR2` must be loaded even when the
fault is later superseded by a `#DF`.

The interpreter's existing IDT path already handles a doubleword error code for
386 gates, so the mechanical part is small. The hard part is not delivery, it is
everything below.

## Where this will actually be hard

1. **Restartability.** `#PF` is a fault, so the instruction re-executes from the
   beginning. Any instruction that has already committed architectural state
   before it faults is a bug that only paging exposes: a `REP MOVS` mid-string,
   a `PUSHA` that faults on the fifth push, a read-modify-write whose write
   faults after its read. The interpreter's existing "nothing commits until the
   whole access can succeed" discipline (used by the `BT` family and by segment
   loads) has to extend to every memory operand once paging can fault. Expect
   this to be the largest part of the round-4 slice, and expect it to need
   targeted tests per instruction family rather than a blanket fix.
2. **Page-crossing accesses.** The engine translates one address. A `u32` read
   at `...FFE` spans two pages, and the *second* page can fault after the first
   byte has been read. Splitting an access at the page boundary, translating
   each half, and detecting the fault on the second half **before** committing
   the first is caller work. The same applies to instruction fetch: a single
   instruction can straddle a page boundary and fault partway through decode.
3. **Fault ordering against everything else.** Segment-limit `#GP`/`#SS` come
   before paging; alignment checks and I/O permission come from other
   mechanisms. Getting `#PF` to fire in the right position relative to those is
   ordering work the engine cannot do, because it never sees the other checks.
4. **Keeping the TLB honest.** The MMU only invalidates when told. `MOV to CR3`
   is the obvious hook, but `CR3` also changes on a task switch and on reset,
   and `CR0.PG` changes through `MOV CR0` and `LMSW`. Route every one of them
   through `Mmu::on_mov_to_*`, or call `Mmu::sync_control_registers(&ctx)` once
   per instruction and let the MMU notice. A missed hook produces a stale
   translation that will look like a random guest crash much later.
5. **`CR4` does not exist yet in the interpreter.** `MOV to CR4` is not
   implemented, and `CPUID` advertises neither `PSE` (`EDX[3]`), `PGE`
   (`EDX[13]`), `PAT` (`EDX[16]`) nor `PSE-36` (`EDX[17]`). Per §4.1.4 a guest
   may set `CR4.PSE` or `CR4.PGE` only when the matching CPUID bit is set, so
   round 4 has to either advertise them — which is now truthful, since 4-MiB
   pages and global pages are implemented — or reject writes that set them.
   Advertising `PSE` without advertising `PSE-36` is the profile the engine
   defaults to, and is the combination to aim for first.
6. **Self-referencing and self-modifying page tables.** A guest may map its own
   page tables into linear space and edit them through the same translations
   they define. The engine reads entries through the caller's memory interface
   on every walk, so this works, but it interacts badly with any future
   translation caching beyond the TLB, and with a JIT that caches decoded
   blocks.
7. **The JIT, later.** The interpreter is the semantic reference, so a JIT must
   reproduce the same faults at the same instruction boundaries, including the
   accessed/dirty writes. Any fast-path that skips a walk has to skip it
   identically.

## Suggested round-4 slice boundaries

Not one slice. In rough dependency order:

1. `MOV to CR3` / `MOV to CR4` plus `CR2` as architectural state, with the
   CPUID bits decided; no translation yet.
2. `CR0.PG` gating one memory-access helper on the paging path, single-page
   accesses only, `#PF` delivery through the existing 386 gate.
3. Page-crossing reads, writes and instruction fetches.
4. Restartability audit per instruction family.
5. `INVLPG` becoming a real invalidation instead of the current real-mode NOP,
   and the TLB hooks on every `CR0` / `CR3` / `CR4` path.

Only after those does "a small 32-bit Linux kernel reaches a serial shell"
become a question about paging rather than about plumbing.
