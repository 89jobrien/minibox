# Pre-Push Commit Range Resolver

**Date:** 2026-04-11
**Status:** Approved

## Problem

Minibox has no `pre-push` hook. The existing `pre-commit` hook handles version bumps and
delegates to a global secret scanner (obfsck), but nothing runs at push time to compute
the set of commits being pushed. Downstream tooling (obfsck integration, CI gating) needs
a reliable commit range before it can act.

## Goal

Add `.githooks/pre-push` that resolves the correct commit range for any push operation and
prints it to stdout. No scanning, no blocking beyond the git protocol itself. obfsck
integration is a follow-on.

## Design

### Input

Git calls the hook with:

```
$1  remote name
$2  remote URL
```

And writes one line per ref to stdin:

```
<local_ref> <local_oid> <remote_ref> <remote_oid>
```

### Range resolution rules

| Condition | Range | Action |
|---|---|---|
| `local_oid == zero` | — | Branch deletion — exit 0, nothing to inspect |
| `remote_oid == zero` | `merge-base..local_oid` | New branch — inspect commits since divergence from `main` |
| otherwise | `remote_oid..local_oid` | Existing branch update — inspect only new commits |

For new branches, the base is `git merge-base <local_oid> main`. If `main` doesn't exist
(e.g. fresh repo, different default branch), fall back to the initial commit:
`git rev-list --max-parents=0 HEAD`.

### Output

Prints the resolved range to stdout, e.g.:

```
abc123def456..789abcdef012
```

One range per ref pushed. If multiple refs are pushed in a single operation, one line per
ref. Exit 0 on success, exit 1 only on unexpected errors (not on empty range).

### Structure

```bash
#!/usr/bin/env bash
set -eou pipefail

fail()           # stderr + exit 1
zero_oid()       # compute git zero OID via hash-object
merge_base()     # git merge-base with main fallback to initial commit
resolve_range()  # core logic — reads stdin, prints range(s)
main()           # entry point, calls resolve_range
```

### Style

- Matches existing `.githooks/pre-commit`: bash, no external deps beyond git
- `set -eou pipefail` throughout
- Color stderr output when connected to a terminal (same `\x1b` pattern as source hook)
- No dependency on `pipeline`, `obfsck`, or any project-specific binary

## Files

| Path | Action |
|---|---|
| `.githooks/pre-push` | Create (new file, executable) |

## Verification

```bash
# Install hooks locally
git config core.hooksPath .githooks

# Simulate push to existing branch (prints remote_oid..local_oid)
git push origin HEAD

# Simulate new branch push (prints merge-base..HEAD)
git checkout -b test/hook-verify
git commit --allow-empty -m "test"
git push origin test/hook-verify

# Simulate delete (should be silent, exit 0)
git push origin --delete test/hook-verify
```
