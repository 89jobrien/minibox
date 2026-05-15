---
name: install-hooks
description: >
  Install git hooks (pre-commit, pre-push, commit-msg) into .git/hooks/.
  Run once after cloning or when hooks are missing.
argument-hint: ""
---

# install-hooks

Writes `pre-commit`, `pre-push`, and `commit-msg` hooks to `.git/hooks/`.

```nu
nu scripts/install-hooks.nu
```

Installed hooks:
- `pre-commit` — runs `just pre-commit` (fmt-check + clippy + release build)
- `pre-push` — runs `just prepush` (nextest + coverage)
- `commit-msg` — warns if message does not follow conventional commits format

Run from the repo root. Existing hooks are overwritten.
