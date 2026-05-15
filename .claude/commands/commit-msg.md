---
name: commit-msg
description: >
  Generate a conventional commit message from staged changes using Claude.
  Use before committing when you want an AI-generated message.
argument-hint: "[--stage] [--commit] [--yes]"
---

# commit-msg

Generates a conventional commit message from the current staged diff using Claude.

```nu
nu scripts/commit-msg.nu                # generate message, print only
nu scripts/commit-msg.nu --stage        # git add -A first, then generate
nu scripts/commit-msg.nu --commit       # generate and prompt to commit
nu scripts/commit-msg.nu --commit --yes # generate and commit without prompting
```
