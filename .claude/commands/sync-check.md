---
name: sync-check
description: >
  Use before git push, when push is rejected (non-fast-forward), or when an LLM agent
  needs to integrate remote changes before continuing. Fetches origin, diagnoses divergence,
  and resolves or escalates.
disable-model-invocation: false
---

# sync-check — Pre-Push Sync

Run before any `git push` to catch divergence early. Also the recovery path when push is
rejected with "non-fast-forward" or "fetch first".

## Step 1 — Run the script

```bash
rust-script scripts/sync-check.rs
# or via Nu wrapper:
nu scripts/sync-check.nu
```

Interprets the output:

| Output contains | Meaning | Action |
|---|---|---|
| `up to date` | Nothing to do | Push is safe |
| `ahead` only | Local commits not on remote | Push is safe |
| `behind` | Remote has commits you don't | Integrate before pushing |
| `diverged` | Both sides have new commits | Requires merge or rebase |

## Step 2 — If behind or diverged: fetch and assess

```bash
git fetch origin
git log --oneline HEAD..origin/main   # what remote has that you don't
git log --oneline origin/main..HEAD   # what you have that remote doesn't
```

## Step 3 — Integrate

**No merge commits on branch** (safe to merge):

```bash
git merge origin/main
```

**Branch has merge commits** — use merge only, never rebase:

```bash
git log --oneline --merges main..HEAD   # confirms merge commits exist
git merge origin/main
```

If conflicts arise, follow the `atelier:merge` skill for resolution.

## Step 4 — Re-run sync-check

```bash
nu scripts/sync-check.nu --dry-run
```

Must show `up to date` or `ahead` before pushing.

## Step 5 — Push

```bash
git push
```

Never use `--no-verify`. If hooks fail, fix them — do not bypass.

## Key Rules

- **Run sync-check before every push** — catches divergence before the hook chain fires
- **Never force-push `main`** — feature branches only, explicit user instruction required
- **Conflicts → use `atelier:merge`** — do not resolve blindly with `--ours`/`--theirs`
