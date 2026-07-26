---
name: audit-plans
description: Cross-check plans and status fields against git log to
    find stale plan markings. Flags plans marked 'done' with no matching commits, and
    plans marked 'open' that appear to have landed. Run before writing new plans or
    at session start to understand true project state.
argument-hint: "path/to/plans/ | --deep | --since=YYYY-MM-DD"
---

## Step 1 — Inventory all plan and spec files with their status

Use Glob to find all `.md` files in:

- `$ARGUMENTS`
- default: `docs/{plans|specs}/*.md`
- possible: `docs/superpowers/{plans|specs}/*.md` | `.ctx/{plans|specs}/*.md`

For each file, read the first 10 lines and extract the `status:` field from YAML
frontmatter. Group results into four buckets: `done`, `open`, `superseded`, `missing`
(no status field found).

Print a compact inventory table: filename | status | title (from `title:` or `#` heading).

---

## Step 2 — Audit `status: done` plans for commit evidence

For each plan with `status: done`:

1. Read the first 30 lines to extract 1–3 short search keywords from the title or
   `deliverables:` / `summary:` fields. Prefer concrete nouns: feature names, crate
   names, or subsystem names (e.g., `daemonbox`, `parallel pull`, `ghcr`, `cgroup`,
   `policy gate`, `image gc`, `bridge network`). Keep each keyword to 1–2 words.

2. For each keyword, run:

    ```bash
    git -C /Users/joe/dev/minibox log --oneline --since="2026-01-01" | grep -i "<keyword>"
    ```

3. If **no keyword** returns any matching commits, flag the plan as **suspicious-done**
   (marked done but no evidence in git log).

4. If at least one keyword matches, record the most recent matching commit SHA+message
   as evidence and mark as **confirmed**.

---

## Step 3 — Audit `status: open` plans for accidental completion

For each plan with `status: open`:

1. Same keyword extraction as Step 2.

2. Run the same git log grep for each keyword.

3. If **any keyword** returns 2 or more matching commits, flag the plan as
   **suspicious-open** (may have already landed).

4. Record the matching commit SHAs as evidence.

---

## Step 4 — Cross-check against HANDOFF.md

Read `/Users/joe/dev/minibox/HANDOFF.md` (full file). Scan for:

- Tasks or blockers listed as open/pending
- Tasks listed as completed

For each open blocker in HANDOFF.md, check whether a plan file covers it and whether
that plan is marked `done`. Flag inconsistencies where HANDOFF.md says open but the
plan says done, or vice versa.

Also read `/Users/joe/dev/minibox/HANDOFF.minibox.workspace.yaml` if present, and
apply the same cross-check to any `status:` fields found there.

---

## Step 5 — Report

Output two tables followed by a summary.

**Table 1: Suspicious `done` plans**

| Plan file                 | Keywords searched | Last matching commit    | Verdict   |
| ------------------------- | ----------------- | ----------------------- | --------- |
| `plans/2026-03-XX-foo.md` | `foo`, `bar`      | `abc1234 feat: add foo` | confirmed |
| `plans/2026-03-XX-baz.md` | `baz`             | none found              | STALE?    |

**Table 2: Suspicious `open` plans**

| Plan file                 | Keywords searched | Matching commits            | Verdict |
| ------------------------- | ----------------- | --------------------------- | ------- |
| `plans/2026-03-XX-qux.md` | `qux`             | `def5678 feat: qux support` | LANDED? |

**Summary line:**

```
Audited N plans/specs. Confirmed done: X. Suspicious done (stale?): Y.
Suspicious open (landed?): Z. HANDOFF inconsistencies: W.
Recommended: re-read flagged plans before writing new work.
```

---

## Notes for executing this skill

- Use Bash for all `git log` calls; use Glob and Read for file discovery and frontmatter
  parsing. Do not use grep as a Bash command — use the Grep tool for content searches
  within files.
- Keep keyword searches short (1–2 words). Full plan titles rarely appear verbatim in
  commit messages.
- The `--since="2026-01-01"` bound covers the full project history; adjust only if the
  project predates 2026.
- Specs (`docs/superpowers/specs/`) are design documents, not delivery plans. Report
  their statuses in the inventory but skip them in the suspicious-done/open analysis
  (they are rarely marked `done`).
- Do not modify any plan files during this audit. Report only.

## Dashbox Logging

After completing the audit, append to `~/.mbx/automation-runs.jsonl`:

```bash
echo '{"run_id":"'$(date -u +%Y-%m-%dT%H:%M:%S)'","script":"plan-audit","status":"complete","duration_s":0,"output":"Audited N plans: M suspicious found"}' >> ~/.mbx/automation-runs.jsonl
```
