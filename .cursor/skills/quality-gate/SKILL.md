---
name: quality-gate
description: Runs formatter, clippy, tests, and optional Wasm/browser checks, then produces the standard done-report. Use before marking a slice complete, opening a PR, or when the user asks if work is finished.
---

# Quality gate

## Run (adapt to what exists)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

When targets exist, also:

```bash
# Wasm package build for emulator-web
# web unit tests / Playwright smoke
# decoder vs XED oracle
# interpreter vs native oracle / JIT lockstep
```

## On failure

1. Fix issues in the current slice only.
2. Re-run the failing command.
3. Do not disable lints or delete tests to force green.

## Done report (required)

```markdown
## Done report
- **Slice:** ...
- **Files changed:** ...
- **Spec sections:** ...
- **Tests added:** ...
- **Commands run:** ...
- **Results:** pass/fail
- **Unsupported remaining:** ...
- **Follow-ups:** ...
```

Only claim done if gates relevant to the touched area are green.
