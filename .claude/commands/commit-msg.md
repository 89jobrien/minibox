---
name: commit-msg
description: >
  Generate a conventional commit message from staged changes using Claude.
  Use before committing when you want an AI-generated message.
argument-hint: "[--stage] [--commit] [--yes]"
allowed-tools: [Bash]
---

Generate a conventional commit message from staged changes.

Parse `$ARGUMENTS` for flags: `-a`/`--stage`, `-c`/`--commit`, `-y`/`--yes`.

1. If `--stage`: run `git add -A`
2. Run `git diff --staged` — error if empty (nothing staged)
3. Generate a conventional commit message:
   - Format: `type(scope): description` (≤72 chars)
   - Types: feat, fix, docs, chore, refactor, test, perf, ci, build, style, revert
   - Scope: the crate or module most affected (e.g. `miniboxd`, `macbox`, `protocol`)
   - Description: imperative mood, lowercase, no trailing period
4. Print the generated message
5. If `--commit`:
   - Show the message
   - If not `--yes`: prompt "Commit with this message? [y/N]"
   - On confirmation: `git commit -m "<message>"`
