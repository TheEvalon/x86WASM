---
name: next-slice
description: Picks the next bounded emulator backlog slice from plan.md and current repo state. Use when starting a vibe-coding session, asking what to build next, or needing a Cursor-ready task prompt with acceptance criteria.
---

# Next slice

## Goal

Produce one small, executable Cursor task — not a milestone rewrite.

## Steps

1. Read `AGENTS.md` and the current milestone in `plan.md` (§21, §24, §25).
2. Inspect the repo: what crates/docs/tests already exist.
3. Choose the **first incomplete** item that unblocks the month-one path:

```text
Reset vector → decoder → interpreter → port I/O → serial/debug → CLI + browser output
```

4. Prefer Epic order A→G in `plan.md` §25 until Milestone 1 exits.
5. Draft a slice with hard boundaries:

```markdown
## Slice: <name>
**Branch:** feat/<kebab-name>
**In scope:**
- ...
**Out of scope:**
- ...
**Spec / docs:**
- ...
**Acceptance tests:**
- [ ] ...
**Commands:**
- cargo test -p <crate> ...
```

6. Stop after presenting the slice. Do not implement unless the user says to proceed.
7. If they proceed, remind them to use Plan Mode first, then the matching skill (`implement-instruction`, `implement-device`, etc.).

## Anti-patterns

- Do not propose "add long mode" or "boot FreeDOS" as a single slice.
- Do not skip documentation ADRs required by Milestone 0 if those files are still missing.
- Do not start JIT, networking, audio, SMP, or Windows work before Milestone 1 exit.
