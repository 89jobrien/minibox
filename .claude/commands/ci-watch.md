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
2. If xtask is unavailable, fallback — aggregate all workflows for the HEAD commit:
   ```bash
   BRANCH=$(git branch --show-current)
   gh run list --branch "$BRANCH" --limit 15 \
     --json databaseId,headSha,workflowName,status,conclusion
   ```
   Filter to runs matching the latest `headSha`, report status for each workflow,
   and watch any that haven't completed.

Output includes: repo, branch, all workflows for the HEAD commit, per-job results
with timing. Exit non-zero if any workflow concluded with failure.
