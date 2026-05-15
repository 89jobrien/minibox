---
name: promote
description: >
  Cascade-merge through the stability pipeline: develop → next → staging → main.
  Use after CI is green on a branch and you want to advance it through the pipeline.
argument-hint: "[--from <branch>] [--to <branch>] [--dry-run]"
allowed-tools: [Bash]
---

Cascade-merge through the stability pipeline. Parse `$ARGUMENTS` for:
`--from <branch>`, `--to <branch>`, `--dry-run`.

Pipeline order: `develop` → `next` → `staging` → `main`

**Rules (enforce these — do not bypass):**
- Do not promote `next` → `staging` without confirming `next` CI is green
- Do not promote `staging` → `main` without confirming `staging` CI is green
- Use `--dry-run` to verify the plan before executing

**Steps:**

1. Record current branch: `git branch --show-current`
2. Stash any `.ctx/` changes: `git stash push -m "promote-stash" -- .ctx/` (if dirty)
3. Determine tier sequence from `--from`/`--to` (default: develop→next→staging→main)
4. In `--dry-run` mode: print the planned merges and exit
5. For each adjacent pair `(source, target)` in the sequence:
   a. `git checkout <target>`
   b. `git merge --ff-only <source>` — abort on non-fast-forward
   c. `git push origin <target>`
6. Pop stash if one was created: `git stash pop`
7. Return to original branch: `git checkout <original>`
