# Plan: minibox-ci-remediation

## Goal

Clear the local repository hazards sitting in the minibox `develop` worktree:
141 unpushed commits on `next`, 19 unreviewed stashes, two already-landed PRs,
and one file that a `git add -A` would commit by accident.

Already done and removed from this plan: the `config.rs` project-layer work
(landed as `c28affbb`) and the single unpushed `develop` commit `a7be1866`
(now on `origin/develop`).

## Architecture

- **Crates affected**: none. No source file in this plan is edited — every task
  is repository hygiene: branches, stashes, PRs, one `.gitignore` line.
- **New traits/types**: none.
- **Data flow**: not applicable. This plan moves existing commits between local
  and remote and edits PR/stash metadata; it introduces no runtime behaviour.

### Findings this plan is built on

| Fact                                                   | Evidence                                                                                                                                                                                                                                                                        |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `next` is a clean fast-forward                         | `git merge-base --is-ancestor origin/next next` → true; 141 ahead, **0** behind                                                                                                                                                                                                 |
| PR #519 is obsolete                                    | `git ls-tree -r origin/develop --name-only \| grep -x book` → no match. `book/` already gone from `develop`                                                                                                                                                                     |
| PR #468 is obsolete                                    | `git ls-tree -r origin/develop --name-only \| grep -x '.github/workflows/publish-mbx.yml'` → match. **The `-r` flag is required** — without it `ls-tree` lists only the top level and this can never match                                                                      |
| `taskit-todo-sync.lock` would be committed by accident | `git check-ignore -v taskit-todo-sync.lock` → no match; no `taskit-todo-sync.lock` line in `.gitignore`                                                                                                                                                                         |
| Full gate is `cargo xtask verify`                      | `cargo xtask` has **no** `fmt` task and `test` requires a suite arg (`unit`, `conformance`). `verify` = fmt + clippy + check + borrow fixtures + docs                                                                                                                           |
| Doc-frontmatter errors are pre-existing                | `cargo xtask verify` prints `error[docs/frontmatter/missing-title]` for `MUTATION_AUDIT.mbx.md`, `ROADMAP.mbx.md`, `SECURITY_INVARIANTS.mbx.md` and then prints `verify gate passed`. `git show HEAD:docs/core/MUTATION_AUDIT.mbx.md` lacks `title:` — present before this plan |

## Tech Stack

- Rust edition: **2024** (`Cargo.toml:28`, inherited by all crates)
- Gate runner: `cargo xtask` — the read-only gate is **`cargo xtask verify`**
  (`fmt-check + clippy + check + borrow fixtures + docs audit`). There is no
  `cargo xtask fmt`. Tests run as `cargo xtask test unit`.
- Tests: `cargo nextest run -p miniboxd`
- Remote: `origin` = `git@github.com:89jobrien/minibox.git`
- No new dependencies. No `Cargo.toml` changes.

## Tasks

### Task 1: Ignore the todo-sync lockfile before it is committed by accident

**Crate**: `n/a` (repo hygiene)
**File(s)**: `.gitignore`
**Run**: `git check-ignore -v taskit-todo-sync.lock`

1. Write the failing check first:

   ```
   git check-ignore -v taskit-todo-sync.lock
   ```

   Expected: FAIL — exits non-zero with no output, because the file is tracked
   as untracked-and-not-ignored and any `git add -A` would sweep it in.

2. Append to `.gitignore`:

   ```
   taskit-todo-sync.lock
   ```

3. Verify GREEN:

   ```
   git check-ignore -v taskit-todo-sync.lock
   ```

   Expected: PASS — prints `.gitignore:NNN:taskit-todo-sync.lock`.

4. `git branch --show-current`
   Verify output is `develop`. Stop immediately if it is not.

   ```bash
   git add .gitignore
   git commit -m "chore: ignore taskit-todo-sync.lock"
   ```

### Task 2: Push the 141 stranded commits on `next`

**Crate**: `n/a` (branch hygiene)
**File(s)**: none
**Run**: `git push origin next`

1. Confirm the fast-forward still holds after Task 1's commit:

   ```
   git fetch origin next && git merge-base --is-ancestor origin/next next
   ```

   Expected: exit 0. If non-zero, `next` has diverged — **stop**, do not push,
   and rebase deliberately instead.

2. Confirm you are not on `main`:

   ```
   git branch --show-current
   ```

   Expected: `develop`. You are pushing a _named branch_, not checking it out.

3. Push:

   ```
   git push origin next
   ```

   Expected: `next -> next` fast-forward, 141 commits.

4. Verify:

   ```
   git rev-list --count origin/next..next
   ```

   Expected: `0`.

   _No commit in this task — it publishes existing commits to the remote. The
   working tree is untouched._

## Already completed (not ingested)

These two were live tasks when this plan was drafted and completed by another
writer while it was under review. They are recorded here for provenance and are
deliberately **not** in `### Task N:` form, so `godmode plan ingest` does not
re-add finished work to the graph.

- **The `config.rs` project-layer work** — landed as `c28affbb`
  (_feat(miniboxd): add a project-level minibox.toml config layer_,
  2026-10-01 17:49). `git status --porcelain crates/miniboxd/src/config.rs` is
  empty; `cargo nextest run -p miniboxd --lib config` at `HEAD` still reports
  **21 passed, 2 skipped**.
- **Pushing the unpushed `develop` commit** — `a7be1866` is now on
  `origin/develop` (`git branch -a --contains a7be1866` → `develop`,
  `remotes/origin/develop`); `git log --oneline @{u}..develop` is empty.
  `develop` has since advanced to `e89acb92`.

### Task 3: Close the two already-landed PRs

**Crate**: `n/a` (PR hygiene)
**File(s)**: none
**Run**: `gh pr close 519 -R 89jobrien/minibox --comment "..."`

1. Re-confirm PR #519 is obsolete before closing:

   ```
   git ls-tree -r origin/develop --name-only | grep -x book
   ```

   Expected: **no output**. The directory it deletes is already gone from
   `develop`. If this prints a match, **stop** — do not close.

   `-r` is required. Without it `ls-tree` lists only the top level of the tree
   and can never match a nested path.

2. Close #519:

   ```
   gh pr close 519 -R 89jobrien/minibox \
     --comment "Superseded: book/ is already absent from develop (verified via git ls-tree -r origin/develop). Closing as already-resolved."
   ```

3. Re-confirm PR #468 is obsolete before closing:

   ```
   git ls-tree -r origin/develop --name-only | grep -x '.github/workflows/publish-mbx.yml'
   ```

   Expected: a match. The workflow it adds is already on `develop`. If this
   prints nothing, **stop** — do not close.

   `-r` is **load-bearing here**. `.github/workflows/publish-mbx.yml` is nested
   three levels deep; non-recursive `ls-tree` will report no match even when the
   file is present, which would read as "not obsolete" and wrongly block the
   close.

4. Close #468:

   ```
   gh pr close 468 -R 89jobrien/minibox \
     --comment "Superseded: publish-mbx.yml is already present on develop (verified via git ls-tree -r origin/develop). Closing as already-landed."
   ```

   _No commit in this task — PR state lives on GitHub, not in the tree._

### Task 4: Record the stash triage decision

**Crate**: `n/a` (hygiene record)
**File(s)**: `.ctx/HANDOFF.md`
**Run**: `git stash list`

1. Enumerate and classify all 19 stashes by whether their base branch commit
   is still reachable:

   ```
   git stash list | cat
   ```

   Expected: 19 entries. Classify each as:
   - **landed** — the content is now in `develop`; safe to drop
   - **orphan** — base branch long gone, content never merged; needs `git stash show -p` review
   - **checkpoint** — explicitly labelled "pre-merge"/"prepush" and superseded by a completed merge

   Known checkpoints from the audit: `stash@{0}` opencode-doc-sync-unrelated,
   `stash@{1}` opavs-doc-audit-fix, `stash@{2}` prepush checkpoint,
   `stash@{3}` handoff before flow auto, `stash@{8}` and `stash@{10}` promote-wip,
   `stash@{12}` pre-merge working tree, `stash@{14}` pre-merge-slot7,
   `stash@{16}` pre-merge local changes, `stash@{18}` wave-integrate auto-stash.

2. **Do not drop any stash in this task.** Write the classification table into
   `.ctx/HANDOFF.md` so a human can approve drops. Dropping stashes is
   destructive and irreversible; the audit's `Never drop git stashes` rule
   applies.

3. Record the ledger:
   ```
   git add .ctx/HANDOFF.md
   git commit -m "chore(handoff): classify 19 develop stashes for review"
   ```

### Task 5: Record the state of the blocked godmode tasks

**Crate**: `n/a` (task graph)
**File(s)**: `.ctx/godmode/tasks.yaml`
**Run**: `godmode task list --json | jq '[.[]|select(.status=="blocked")]'`

The task graph is read **only** through the `godmode` CLI. It is backed by
`.ctx/godmode/tasks.yaml` (132 tasks, mtime 2026-10-01). A second,
**stale** file `.ctx/GODMODE.tasks.yaml` (170 tasks, mtime 2026-08-22) also
exists and is _not_ read by the CLI — ignore it.

1. List the blocked tasks from the CLI:

   ```
   godmode task list --json | jq -r '.[]|select(.status=="blocked")|"  \(.id) :: \(.title)"'
   ```

   Expected: three entries —

   ```
     gh-473 :: feat(xtask): squash chain branches during promotion
     gh-507 :: feat(oci): stabilize push, commit, and build
     2026-09-22-signed-governed-execution-daemon-t1 :: Pin the immutable Crux contract
   ```

   A different count means the graph shifted — **stop** and re-read it.

2. Record the blocked set in `.ctx/HANDOFF.md` so it survives the session.
   `gh-473` and `gh-507` are blocked on CI gates that this plan does not verify
   (see _Out of scope_), so **leave both `blocked`**. Do not run
   `godmode task unblock` on either — clearing them would assert a green gate
   state that has not been established.

3. Note the third blocked task,
   `2026-09-22-signed-governed-execution-daemon-t1` (_Pin the immutable Crux
   contract_). It is **not** referenced in `.ctx/HANDOFF.md` and its blocker is
   not recorded anywhere this plan can verify. Flag it for triage; do not
   unblock or complete it.

4. The graph also holds 63 `pending` tasks, dominated by four families:
   `gh-492`–`gh-518`, `iso-profile-*`, `govexec-1`–`govexec-10`, and
   `govplugin-1`–`govplugin-9`. Confirm the count without acting on it:

   ```
   godmode task list --json | jq '[.[]|select(.status=="pending")]|length'
   ```

   Expected: `63`. Real work — **out of scope for this plan**, which touches
   only the blocked set.

5. `git branch --show-current`
   Verify output is `develop`. Stop immediately if it is not.

   `.ctx/godmode/tasks.yaml` is gitignored (`.gitignore:166` → `.ctx/*`), so
   nothing here is committed:

   ```
   git check-ignore -v .ctx/godmode/tasks.yaml
   ```

   Expected: a match. _No commit in this task — the file is gitignored by
   design._

## Post-plan verification

Run after Task 5:

```bash
cd /Users/joe/dev/minibox
git status --porcelain .gitignore      # expect: clean (the lock line is committed)
git check-ignore -v taskit-todo-sync.lock   # expect: a match on .gitignore
git rev-list --count origin/next..next # expect: 0
git stash list | wc -l                 # expect: still 19 (nothing dropped without approval)
git rev-list --count @{u}..develop     # expect: 0
gh pr list -R 89jobrien/minibox --state open
```

`taskit-todo-sync.lock` should no longer appear in `git status --porcelain`.
PR #519 and #468 should be gone from the open list; #565 remains open.

**Do not** assert on total `git status --porcelain` count. The tree is under
concurrent modification (see below) and the `site/` churn is owned by another
writer.

## Out of scope

CI gate state is **not** verified or fixed by this plan. As of
2026-10-01T08:30Z, minibox nightly and seven other repos show failing runs
whose jobs complete in 2–4s with zero steps executed, and `notfiles-config` /
`sitex` report `startup_failure`. The cause is unconfirmed and deliberately not
investigated here. Consequences for this plan:

- No task depends on a green remote check. All verification is local
  (`git ls-tree`, `git rev-list`, `git check-ignore`, `gh pr view`).
- `gh-473` and `gh-507` stay `blocked` (Task 5).
- PR #565's CI will keep failing until that separate issue is resolved.

## Concurrent-writer warning

This repo was under active modification while this plan was written and
verified. Between authoring and verification, `develop` advanced through four
commits (`47d66511`, `c28affbb`, `e89acb92`), and the working tree changed from
18 dirty files to 20 — all `site/` churn unrelated to this plan.

Every numeric expectation in this plan is a **snapshot**. Re-run the task's own
verification command before trusting it; if a precondition has drifted, the task
tells you to stop rather than proceed on a stale value.
