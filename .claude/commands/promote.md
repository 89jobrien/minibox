---
name: promote
description: >
  Run quality gates and cascade-merge through the stability pipeline:
  develop -> staging -> release -> main via cargo-promote.
argument-hint: "[--from <tier>] [--to <tier>] [--dry-run]"
allowed-tools: [Bash]
---

Run `cargo xtask promote` with the user's `$ARGUMENTS`. `--from`/`--to`
take branch names directly: `develop` -> `staging` -> `release` -> `main`.

The xtask runs quality gates for each tier, then calls
`cargo-promote branch` to cascade-merge the git branches.

**Rules (enforce these -- do not bypass):**

- Do not promote without confirming source branch CI is green
- Use `--dry-run` to verify the plan before executing

**Example:**

```bash
cargo xtask promote --from develop --to staging --dry-run
cargo xtask promote --from develop --to main
```
