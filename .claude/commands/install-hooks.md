---
name: install-hooks
description: >
  Install git hooks (pre-commit, pre-push, commit-msg) into .git/hooks/.
  Run once after cloning or when hooks are missing.
argument-hint: ""
allowed-tools: [Bash, Write]
---

Install git hooks into `.git/hooks/`. Run from the repo root.

Error if `.git/hooks/` does not exist.

Write these three files (overwrite if present):

**pre-commit** — runs fmt-check + clippy + release build:
```sh
#!/bin/sh
set -e
just pre-commit
```

**pre-push** — runs nextest + coverage:
```sh
#!/bin/sh
set -e
just prepush
```

**commit-msg** — warns if message does not match conventional commits pattern:
```sh
#!/bin/sh
MSG=$(cat "$1")
echo "$MSG" | grep -qE "^(Merge |Revert )" && exit 0
if ! echo "$MSG" | grep -qE "^(feat|fix|docs|chore|refactor|test|perf|ci|build|style|revert)(\(.+\))?: .{1,72}"; then
    echo "warning: commit message does not follow conventional commits format"
    echo "  expected: type(scope): description"
fi
exit 0
```

Set executable: `chmod +x .git/hooks/pre-commit .git/hooks/pre-push .git/hooks/commit-msg`
