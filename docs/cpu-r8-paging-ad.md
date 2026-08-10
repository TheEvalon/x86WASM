# Round 8 — Paging accessed/dirty honesty + INVLPG (PE=1)

## Status

Accessed/dirty updates on **successful** walks were already correct after
Round 4 (`docs/cpu-r4-paging-integration.md`, `cpu_r4_paging_data_path.rs`):

* A successful read sets `A` in every entry used; a write also sets `D` in the
  final mapping entry (Vol. 3 §4.8).
* Bit 6 of a PDE that references a page table is ignored and never written.
* A faulting walk writes **no** paging-structure byte, including higher-level
  `A` — the documented §4.10.2.3 honesty choice (cache/A only after a
  translation that completes). **Not** changed in this slice.

This slice adds PE=1 / PG=1 coverage that `INVLPG` through `PagedBus` drops
exactly the addressed page so a remapped PTE becomes visible, and that a
CPL≠0 `INVLPG` raises `#GP(0)` without invalidating.

## Spec

Intel SDM Vol. 3 §§4.8, 4.10.2.3, 4.10.4.1; Vol. 2 "INVLPG".

## Files

* `crates/x86-interpreter/tests/cpu_r8_paging_ad.rs`
* `docs/cpu-r8-paging-ad.md`

No interpreter behavioral change was required for A/D; INVLPG already called
`Bus::invalidate_page` (wired by `PagedBus` to the MMU TLB).
