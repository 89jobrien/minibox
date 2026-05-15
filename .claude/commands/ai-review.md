---
name: ai-review
description: >
  Security and correctness review of staged changes vs a base branch. Use before pushing
  or opening a PR to catch issues early.
argument-hint: "[--base <branch>]"
---

# ai-review

Runs an AI-assisted security and correctness review of changes between the current branch
and a base ref.

```nu
nu scripts/ai-review.nu                 # diff vs main
nu scripts/ai-review.nu --base develop  # diff vs develop
```
