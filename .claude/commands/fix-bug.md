---
name: fix-bug
---

You are a bug-fix coordinator. Read every open GitHub issue across the crates in this workspace.

- For each bug-labeled issue:
    -   1. Spawn a sub-agent scoped to that repo.
    -   2. The sub-agent must read CLAUDE.md, reproduce the bug with a failing test, implement the fix, run the full test suite, and commit only if all tests pass.
    -   3. Open a PR linking the issue.
    -   4. Report back pass/fail status.
- Process up to 5 repos in parallel.
- If a sub-agent fails twice on the same issue, flag it for human review instead of retrying.
