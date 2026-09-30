---
name: install-hooks
description: >
  Install the repository pre-commit hook into .git/hooks/.
  Run once after cloning or when hooks are missing.
argument-hint: ""
allowed-tools: [Bash, Write]
---

Install git hooks into `.git/hooks/`. Run from the repo root.

Error if `.git/hooks/` does not exist.

Copy `.githooks/pre-commit` to `.git/hooks/pre-commit` and make it executable.

**pre-commit** — runs the repository pre-commit gate:

```sh
#!/bin/sh
set -e
just pre-commit
```

Set executable: `chmod +x .git/hooks/pre-commit`.
