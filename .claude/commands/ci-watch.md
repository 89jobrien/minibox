---
name: ci-watch
description: >
  Watch the latest GitHub Actions run with job-level detail. Defaults to current branch.
  Use when you need to monitor CI after a push or check another branch's status.
argument-hint: "[--branch <name>]"
---

# ci-watch

Watch the latest GHA run for the current branch (or a specified branch) with job-level
detail and live tail.

```nu
nu scripts/ci-watch.nu                  # current branch
nu scripts/ci-watch.nu --branch main    # specific branch
```

Equivalent xtask:

```bash
cargo xtask ci-watch
cargo xtask ci-watch --branch main
```

Output includes: repo, branch, workflow, trigger, commit SHA, started time, status, and
per-job results with timing. Exits non-zero if the run concluded with failure.
