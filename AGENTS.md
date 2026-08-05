# AGENTS.md — x86WASM Emulator

This repository is built primarily with Cursor agents ("vibe coded"). Read this file before any implementation work.

## Mission

Build a browser-capable full-system x86/x64 PC emulator (Rust core → Wasm) that can eventually boot Windows 10 x64. See [plan.md](plan.md) for the full product and engineering plan.

## Non-negotiables

1. **Spec before code** — Intel manuals and `docs/sources.md` are authoritative. Never invent opcode, flag, paging, or device behavior.
2. **No code theft** — Do not copy implementation from v86, QEMU, Bochs, VirtualBox, DOSBox, or other emulators. Oracles and specs are fine; their source is not.
3. **64-bit state from day one** — GPRs and architectural addresses are `u64`-capable even while only real mode runs.
4. **Interpreter is truth** — JIT must match interpreter architectural state. Browser APIs stay out of core crates.
5. **Truthful CPUID** — Never advertise an unimplemented feature.
6. **One slice per session** — Bound every change to a single issue with explicit acceptance tests. No "implement x86-64" prompts.

## Read order for a new session

1. This file (`AGENTS.md`)
2. Active milestone section in [plan.md](plan.md) (§21 / §24 / §25)
3. Relevant docs under `docs/` (create if missing before coding that area)
4. Matching `.cursor/rules/*.mdc` (auto-applied by Cursor)
5. The skill for the task type under `.cursor/skills/`

## Project skills (invoke explicitly)

| Skill | Use when |
|---|---|
| `next-slice` | Choosing the next bounded backlog item |
| `implement-instruction` | Adding/fixing decode + interpreter semantics for opcodes |
| `implement-device` | Implementing a device register/feature slice |
| `guest-boot-debug` | Debugging firmware/OS boot failures |
| `quality-gate` | Finishing a slice: fmt, clippy, tests, report |

## Vibe-coding session shape

```text
1. Invoke next-slice (or name a tiny issue)
2. Plan Mode: file list + acceptance tests only
3. Agent Mode: implement that slice only
4. Invoke quality-gate
5. Stop. Open a new chat for the next slice.
```

Prefer **new chats per slice** over long multi-topic threads. Prefer **git worktrees** when parallel agents would touch the same crates.

## Quality gates before "done"

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# plus: Wasm build, browser tests, oracle diffs when those exist
```

## Current phase focus

Milestone 0/1 exit (HELLO ROM path) is met. Active work is **Milestone 2** via one bounded slice per chat: real-mode foundation → interrupts/exceptions → legacy PC devices → SeaBIOS/FreeDOS. **Not** VGA/IDE/SeaBIOS as a single slice; **not** Windows, JIT, networking, audio, or SMP.

## Plan authority

[plan.md](plan.md) is the product and roadmap source of truth. If code and plan disagree, fix the smaller delta and update docs in the same change when behavior is intentional.

## MCP token-saving policy

Prefer structured MCP tools over dumping source into chat. Details live in `.cursor/rules/mcp-token-budget.mdc`.

| MCP | Use for | Avoid |
|---|---|---|
| **jcodemunch** | After index: `search_symbols` / `get_context_bundle` / `get_file_outline` / `assemble_task_context` | Broad Grep/Read/Glob of whole crates |
| **cursor10x-mcp** | `initConversation`, milestones/decisions/requirements, `getComprehensiveContext` at boundaries | Re-pasting milestone status every chat |
| **Context7** | Rust, wasm-bindgen, Vite, Playwright docs | Guessing current API shapes |
| **desktop-commander** | Long builds/tests; surgical file ops | Replacing git/`cargo` for normal flow |
| **user-memory** | Durable cross-project facts only if needed | Duplicating `plan.md` |

Session start:

```text
cursor10x initConversation
→ jcodemunch get_session_context / assemble_task_context (once indexed)
→ next-slice skill
→ Plan Mode → implement → quality-gate
→ cursor10x storeMilestone / endConversation
```

Re-index with jcodemunch after each meaningful merge (or when symbols drift).

## Ongoing cadence (after Milestone 1)

Follow [plan.md](plan.md) milestones 2→7. Each session:

1. Invoke `next-slice` against §21 / §25
2. Plan Mode → implement one slice → `quality-gate`
3. Push a branch → open a PR to `main` on [TheEvalon/x86WASM](https://github.com/TheEvalon/x86WASM)
4. Record a cursor10x milestone; re-index jcodemunch when crate/layout symbols change

Do not expand a chat into the next epic. Prefer a new chat (and a git worktree when parallel agents would touch the same crates).
