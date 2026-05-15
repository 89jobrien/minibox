---
name: ai-review
description: >
  Security and correctness review of staged changes vs a base branch. Use before pushing
  or opening a PR to catch issues early.
argument-hint: "[--base <branch>]"
agent: atelier:sentinel
allowed-tools: [Bash, Read]
---

Security and correctness review of changes between the current branch and a base ref.

Parse `$ARGUMENTS` for: `--base <branch>` (default: `main`).

1. Run `git diff <base>...HEAD` to get the full diff
2. If diff is empty, check `git diff --staged` instead; error if both empty

**Review checklist — flag every violation with severity:**

- `[BLOCKING]` Path traversal: user input passed to `Path::join` without `validate_layer_path()`
- `[BLOCKING]` `.unwrap()` in non-test production code — must use `.context()?`
- `[BLOCKING]` `fork()`/`clone()` called directly inside `async fn` — must use `spawn_blocking`
- `[BLOCKING]` `println!`/`eprintln!` in daemon code — must use `tracing::info!/warn!`
- `[BLOCKING]` New serde struct fields without `#[serde(default)]` on wire types
- `[BLOCKING]` `unsafe` block without a SAFETY comment documenting the invariant
- `[WARN]` Missing error context on `?` operators in production paths
- `[WARN]` Missing cleanup on error paths that create resources (overlays, cgroups)
- `[WARN]` Structured log values embedded in message string instead of key=value fields
- `[NIT]` Style or naming inconsistencies with existing code patterns

**Output format:**

For each finding:
```
[SEVERITY] file.rs:42 — description
  Suggested fix: ...
```

Print a summary at the end: N blocking, N warnings, N nits.
Exit non-zero if any BLOCKING issues found.
