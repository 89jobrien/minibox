---
name: ci-watch
description: >
  Watch the latest GitHub Actions run with job-level detail. Defaults to current branch.
  Use when you need to monitor CI after a push or check another branch's status.
argument-hint: "[--branch <name>]"
allowed-tools: [Bash]
---

Run CI watch for the current or specified branch.

1. Run `cargo xtask ci-watch` (pass `--branch <name>` if provided in `$ARGUMENTS`)
2. If xtask is unavailable, fallback:
   ```bash
   gh run watch $(gh run list --branch $(git branch --show-current) \
     --limit 1 --json databaseId --jq '.[0].databaseId')
   ```

Output includes: repo, branch, workflow, trigger, commit SHA, started time, status, and
per-job results with timing. Exit non-zero if the run concluded with failure.
