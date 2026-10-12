---
status: draft
sources:
  - xtask/src/docs_audit.rs
  - xtask/src/docs_lint.rs
  - xtask/src/feature_matrix_date.rs
  - xtask/src/gates.rs
  - xtask/src/main.rs
  - .claude/skills/docs/scripts/doc-audit.nu
  - .claude/skills/docs/SKILL.md
  - taskit.toml
generated: 2026-10-04
---

# Design: Derived Doc Freshness Gate

## Goal

Make documentation freshness _derived from git and blocking_, so a doc whose declared
sources have moved on can no longer sit silently rotting; delete the `source_sha` scalar,
which was the one field that could not be maintained honestly and had rotted on 35 of the
38 stamped docs.

## Approved Approach

Derived-from-git staleness, replacing the existing dead freshness check rather than
adding a second one, with a dedicated hard-failing check wired into `cargo xtask verify`.

The stored per-source SHA map approved in brainstorm was dropped during design: a map
written by a pre-commit hook is necessarily one commit stale, because the commit that
would make it correct does not exist yet. `regenerated_from` survives only as an
advisory record of the last real regeneration, written by whoever regenerates a doc, and
read by nothing that can fail.

## Context Map

### Files to Modify

| File                                       | Purpose                                 | Changes Needed                                                                                                                                                                                                                                                   |
| ------------------------------------------ | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `xtask/src/frontmatter.rs`                 | (new) line-oriented frontmatter parsing | New module: `frontmatter_block`, `frontmatter_scalar`, `frontmatter_list`, `frontmatter_mapping`                                                                                                                                                                 |
| `xtask/src/docs_audit.rs`                  | docs audit (taskit protocol surface)    | Delete `check_freshness`, `parse_watches`, `FreshnessResult`; add `GitHistory` trait, `ShellGitHistory`, staleness types, `check_staleness`, `enforce_staleness`; rewire `--full` freshness section                                                              |
| `xtask/src/docs_lint.rs`                   | frontmatter lint                        | Delete private `extract_frontmatter` / `frontmatter_value`; delegate to `frontmatter.rs`. Widen the walk from `docs/superpowers/{plans,specs}/` to `docs/` recursively, and repair the two rules that the widened walk exposes (see Widening the Docs Lint Walk) |
| `xtask/src/main.rs`                        | xtask CLI                               | Declare `mod frontmatter;`; add `--stale` to the `docs audit` action; update usage text                                                                                                                                                                          |
| `xtask/src/gates.rs`                       | gate orchestration                      | Call `docs_audit::enforce_staleness` in `verify` as a hard failure                                                                                                                                                                                               |
| `xtask/schema/cli.schema.json`             | xtask CLI schema                        | Document the `--stale` flag                                                                                                                                                                                                                                      |
| 38 stamped docs                            | provenance frontmatter                  | Delete the `source_sha:` line                                                                                                                                                                                                                                    |
| 18 living docs (see Migration)             | reference documentation                 | Regenerate content from current code                                                                                                                                                                                                                             |
| `.claude/skills/docs/scripts/doc-audit.nu` | docs skill audit                        | Delegate freshness to `cargo xtask docs audit --stale`; keep collisions and link checks                                                                                                                                                                          |
| `.claude/skills/docs/SKILL.md`             | docs skill contract                     | Rewrite the provenance section; drop the `source_sha` scalar                                                                                                                                                                                                     |
| `AGENTS.md`                                | agent guidance                          | Correct "stamps every generated doc with YAML frontmatter recording the source commit SHA it was derived from"                                                                                                                                                   |
| `docs/core/DOCS_AUDIT.mbx.md`              | audit documentation                     | Describe the derived rule and the new flag                                                                                                                                                                                                                       |
| `docs/core/XTASK_CLI.mbx.md`               | CLI reference                           | Add `--stale`; correct freshness description                                                                                                                                                                                                                     |
| `taskit-protocol.lock`                     | protocol surface hashes                 | Regenerate via `taskit protocol drift --update`                                                                                                                                                                                                                  |

### Dependencies (may need updates)

| File                               | Relationship                                                                   |
| ---------------------------------- | ------------------------------------------------------------------------------ |
| `xtask/src/main.rs`                | sole caller of `docs_audit::run` and `docs_lint::lint_docs` (lines 554, 564)   |
| `xtask/src/gates.rs`               | calls `docs_audit::run` (line 197) and `docs_lint::lint_docs` (lines 194, 292) |
| `xtask/src/feature_matrix_date.rs` | owns `pub fn is_stamp_line`, reused by the substantive-change walk             |
| `xtask/src/sarif.rs`               | `Diagnostic` / `Level` used by `docs_lint`                                     |

No consumer of `xtask/docs-audit-report.json` exists outside documentation
(`docs/core/DOCS_AUDIT.mbx.md`, `docs/core/XTASK_CLI.mbx.md`,
`xtask/schema/cli.schema.json`). The `.worktrees/` copies of `gates.rs` and `main.rs` are
separate branches and are out of scope.

### Reference Patterns

| File                                                           | Pattern to Follow                                                                                      |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `xtask/src/feature_matrix_date.rs`                             | `is_stamp_line` predicate reused verbatim; `StampOutcome` tally-and-report shape for `StalenessReport` |
| `xtask/src/gates.rs` (`staged_change_beyond_stamp`, line 1886) | the stamp-exclusion predicate, generalised from index-diff to commit history                           |
| `xtask/src/docs_audit.rs` (`AuditReport`, `FreshnessResult`)   | `#[derive(Debug, Serialize)]` plus `#[serde(skip_serializing_if = ...)]` for the JSON report           |
| `xtask/src/gates.rs` tests (line 1191)                         | `tempfile::tempdir()` fixtures for filesystem and git-dependent tests                                  |

### Dead Gates Found

Two existing checks never execute. Both are load-bearing context for this design, because
together they explain why 35 stale stamps accumulated unnoticed.

1. `check_freshness` (`docs_audit.rs:306`) reads a `watches:` frontmatter key. **No doc in
   the repo has one** — the corpus uses `sources:`. The function therefore returns an empty
   vector for every doc, and `--full` mode's freshness section has never reported anything.
   It also compared author dates (`--format=%aI`) against commit-count thresholds
   (`>15` stale, `>5` aging), which is stamp-sensitive in the same way the new rule must
   not be.
2. `lint_docs` (`docs_lint.rs:21`) walks only `docs/superpowers/{plans,specs}/`. That
   directory does not exist, so the check reports 0 files and always passes — in
   `pre-commit` (`gates.rs:292`) and in `verify` (`gates.rs:194`) alike.

This design replaces the first and revives the second, by widening its walk to the real
corpus.

### Widening the Docs Lint Walk

`lint_docs` changes from a 0-file walk to a recursive walk over `docs/`, which is 120
markdown files today. Two of its rules were never exercised against a real corpus and
both fail immediately at that scale, so reviving the gate requires repairing them in the
same change. Measured, not estimated:

| Rule                              | Violations across `docs/**/*.md` | Cause                                                                                                                                                           |
| --------------------------------- | -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs-lint/malformed-frontmatter` | 467                              | Rule 1 requires every non-blank frontmatter line to contain a colon, so every YAML list item (`  - crates/macbox`) is an error. No doc in the repo is list-free |
| `docs-lint/invalid-status`        | 11                               | `ALLOWED_STATUSES` is `open, done, archived, approved, draft`; the corpus also uses `proposed` (8), `superseded` (2), `partial` (1)                             |

Both repairs are rule-level, not corpus-level:

- Rule 1 accepts the strict subset that `frontmatter.rs` already parses — `key: value`,
  `  - list item`, and one level of `  key: value`. Anything else remains an error, so the
  rule keeps its meaning instead of being weakened into a no-op.
- `ALLOWED_STATUSES` gains `proposed`, `superseded`, and `partial`. These are real
  lifecycle states in use across `docs/plans/` and `docs/ideas/`; rewriting 11 docs to a
  different status would invent semantics those docs do not have.

`lint_docs` keeps its signature. Files directly under `docs/` are included alongside
subdirectories. Four stamped docs sit outside the walk by construction — `USAGE.md`,
`TESTING.md`, `DEVELOPMENT.md`, and `.claude/skills/docs/SKILL.md` — and their shape is
validated by the contract-based path in `check_staleness` instead, so the shape rules have
exactly one implementation and two entry points.

With those repairs, the legacy-`source_sha` and `regenerated_from` rules below are
enforced in `pre-commit` as well as `verify`, which is strictly better than the alternative
of leaving the shape guard in a code path only `verify` reaches.

## Crate Ownership

- **Owner crate**: `xtask` — it already owns every gate, the docs audit, the frontmatter
  lint, and the CLI; the change adds no runtime code to any shipped crate.
- **Affected crates**: none. `xtask` is `publish = false` with a single `[[bin]]` target
  and no lib target, so there is no public API surface and no semver implication.

## Public API

All items below are crate-internal to the `xtask` binary. Signatures only.

### Traits

```rust
/// Git queries the staleness rule depends on, injectable so the
/// substantive-change walk is testable without a repository on disk.
pub trait GitHistory {
    fn is_shallow_repository(&self) -> Result<bool>;
    fn last_change(&self, path: &str) -> Result<Option<String>>;
    fn commit_subject(&self, sha: &str) -> Result<String>;
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;
    fn commits_touching(&self, path: &str) -> Result<Vec<String>>;
    fn commit_changed_lines(&self, sha: &str, path: &str) -> Result<Vec<String>>;
}
```

### Types

```rust
/// `ShellGitHistory` runs the trait's queries through `xshell::cmd!`.
pub struct ShellGitHistory {
    pub sh: Shell,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StaleReason {
    /// A declared source has a commit after the doc's last substantive change.
    SourceChangedAfterDoc,
    /// A declared source path does not exist in the working tree.
    SourcePathMissing,
    /// A declared source path exists but has no commits.
    SourceUntracked,
    /// The doc's history is exhausted by stamp-only commits, or the walk
    /// exceeded `MAX_SUBSTANTIVE_WALK` before a substantive commit was found.
    IndeterminateHistory,
}

#[derive(Debug, Serialize)]
pub struct StaleSource {
    pub path: String,
    pub last_change: String,
    pub last_change_subject: String,
}

#[derive(Debug, Serialize)]
pub struct StaleDoc {
    pub doc: String,
    pub reason: StaleReason,
    pub sources: Vec<StaleSource>,
}

#[derive(Debug, Serialize)]
pub struct StalenessReport {
    pub checked_docs: usize,
    pub exempt_docs: usize,
    pub untracked_docs: usize,
    pub stale: Vec<StaleDoc>,
}
```

`AuditReport.freshness` changes from `Vec<FreshnessResult>` to `Vec<StaleDoc>`; the JSON
key `freshness` is preserved so the report file's shape stays recognisable.

### Functions

```rust
// xtask/src/docs_audit.rs
pub fn check_staleness(sh: &Shell, root: &Path) -> Result<StalenessReport>;
pub fn enforce_staleness(sh: &Shell, root: &Path) -> Result<()>;
fn substantive_last_change(
    history: &dyn GitHistory,
    doc_rel: &str,
) -> Result<Option<String>>;

// xtask/src/frontmatter.rs
pub fn frontmatter_block(content: &str) -> Option<&str>;
pub fn frontmatter_scalar<'a>(fm: &'a str, key: &str) -> Option<&'a str>;
pub fn frontmatter_list(fm: &str, key: &str) -> Vec<String>;
pub fn frontmatter_mapping(fm: &str, key: &str) -> Vec<(String, String)>;
pub fn validate_provenance_contract(fm: &str, rel: &str) -> Vec<ContractViolation>;

// xtask/src/docs_audit.rs
pub fn is_exempt_path(rel: &str) -> bool;
pub const MAX_SUBSTANTIVE_WALK: usize = 50;
pub const FRESHNESS_EXEMPT_PREFIXES: [&str; 2];

// xtask/src/docs_lint.rs
pub const ALLOWED_STATUSES: &[&str];
```

### Contract Violations

The shape rules are one shared function with two entry points, so `pre-commit` (via
`docs_lint`) and `verify` (via `check_staleness`) cannot drift apart on what the contract
is:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractViolation {
    /// The deleted `source_sha` scalar has returned.
    LegacySourceSha,
    /// A `regenerated_from` key is not declared in `sources`.
    UndeclaredRegeneratedSource { key: String },
    /// A `regenerated_from` value is not a 40-character hex SHA.
    MalformedRegeneratedValue { key: String, value: String },
}
```

`ALLOWED_STATUSES` becomes
`["open", "done", "archived", "approved", "draft", "proposed", "superseded", "partial"]`.

`docs_lint::lint_docs` keeps its signature; only its walk and rule set change.

`is_exempt_path` matches `docs/designs/` and `docs/plans/`: those are point-in-time
records of a decision, and requiring them to track current code is a category error. The
exemption is a named, documented constant rather than an inferred skip, so the next reader
sees a decision instead of an oversight.

### Frontmatter Contract

```yaml
---
status: draft
sources:
  - crates/minibox-core/src/protocol.rs
  - xtask/src/docs_audit.rs
regenerated_from: # optional; absent means "never recorded"
  crates/minibox-core/src/protocol.rs: 4add60894f29527483dd1200aae362faad78f589
generated: 2026-10-04
---
```

- `source_sha` is **deleted**, never backfilled. It is the value that could not be
  maintained honestly, and nothing reads it after this change.
- `sources:` is unchanged in shape and remains the declaration of what the doc covers.
  Entries may be files or directories; `git log -1 -- <path>` handles both.
- `regenerated_from` is advisory. Written only by whoever regenerates a doc. Absence is a
  valid, common state and is never a finding.
- `generated:` is untouched.

xtask has no YAML dependency, so `frontmatter.rs` parses the strict line-oriented subset
only: `key: value`, `  - list item`, and `  key: value` for one level of mapping. Nested
maps, lists under a mapping key, and quoted keys are **shape errors**, not cases to guess
at. This matches the existing `docs_lint` posture, which already rejects any frontmatter
line that is not a `key: value` pair — widened to also accept the list and one-level
mapping forms that `sources:` and `regenerated_from:` actually use.

## Data Flow

1. **Source**: git history for each doc and each declared source, plus each doc's
   `sources:` frontmatter.
2. **Transform**: `substantive_last_change` walks `commits_touching` newest-first and
   skips commits whose `commit_changed_lines` are all `is_stamp_line` hits, capped at
   `MAX_SUBSTANTIVE_WALK`; `last_change` per source; a source whose SHA is a strict
   descendant of the doc's substantive SHA yields `StaleSource`, aggregated into `StaleDoc`.
3. **Sink**: (a) `verify` hard-fails via `enforce_staleness`; (b)
   `cargo xtask docs audit --stale` prints the report as JSON on stdout for
   `doc-audit.nu`; (c) `--full` writes the report into `xtask/docs-audit-report.json`.
4. **Shape sink**: `validate_provenance_contract` emits `ContractViolation`s from either
   `docs_lint` (every `docs/**/*.md`, failing `pre-commit`) or `check_staleness`
   (contract-based, covering the four stamped docs outside `docs/` and failing `verify`).

## Hexagonal Boundaries

- **Port** (trait): `GitHistory` in `xtask::docs_audit`. The rule depends on six git
  queries and nothing else, so the walk's logic is exercised by an in-memory fake.
- **Adapter** (impl): `ShellGitHistory` in `xtask::docs_audit`, backed by `xshell::cmd!`.

The frontier is the trait; everything above it is pure decision logic over strings.

### Staleness Rule

Doc `D` with declared sources `S(D)` is stale when some `s` in `S(D)` has `last_change(s)`
strictly after `substantive_last_change(D)`.

Excluding stamp-only commits from the doc's own history is load-bearing, not a
refinement: `restamp_staged_docs` (`gates.rs:312`) rewrites the body stamp and re-stages
it, so stamp-only doc commits are routine and recent. If those counted as substantive,
almost every doc's last change would be the newest commit in the repository and every doc
would read fresh — inverting the check into a permanent pass.

### Outcomes

No outcome may silently read fresh:

| Condition                                                | Result                                                  |
| -------------------------------------------------------- | ------------------------------------------------------- |
| Source has a commit after the doc's substantive change   | `SourceChangedAfterDoc`                                 |
| Declared source path absent from the working tree        | `SourcePathMissing`                                     |
| Declared source path present but has no commits          | `SourceUntracked`                                       |
| History exhausted by stamp-only commits, or walk cap hit | `IndeterminateHistory`                                  |
| Doc not under version control yet                        | counted in `untracked_docs`, not a finding              |
| Doc with no `sources:` key                               | not collected; 89 such docs today, mostly `docs/src/**` |
| Path matches `FRESHNESS_EXEMPT_PREFIXES`                 | counted in `exempt_docs`, not collected                 |
| `git rev-parse --is-shallow-repository` is true          | `check_staleness` returns an error                      |

CI checks out with `fetch-depth: 0` in all six workflows, so full history is available
there. A shallow local clone would make `git log` silently short and every doc read fresh,
so the check refuses rather than reporting a false pass.

## Test Coverage

Run with `cargo xtask test unit` (the repo is nextest-based; bare `cargo test` is not a
gate). Cases live in the existing `mod tests` blocks: `docs_audit.rs:617`,
`docs_lint.rs:155`, and a new `mod tests` in `frontmatter.rs`.

| Case                                          | Asserts                                                                |
| --------------------------------------------- | ---------------------------------------------------------------------- |
| stamp-only commit skipped by the walk         | `substantive_last_change` returns the commit before the stamp-only one |
| doc edited after its source                   | not reported stale                                                     |
| source edited after the doc                   | reported `SourceChangedAfterDoc` with the source's subject             |
| walk cap exceeded                             | `IndeterminateHistory`, not a pass                                     |
| shallow repository                            | `check_staleness` returns an error                                     |
| missing source path                           | `SourcePathMissing`                                                    |
| untracked source                              | `SourceUntracked`                                                      |
| untracked doc                                 | counted in `untracked_docs`, absent from `stale`                       |
| `docs/designs/`, `docs/plans/`                | `is_exempt_path` true; counted in `exempt_docs`                        |
| doc without `sources:`                        | not collected                                                          |
| legacy `source_sha` scalar present            | shape error, fails the check                                           |
| `regenerated_from` key absent from `sources`  | shape error, fails the check                                           |
| `regenerated_from` value not 40 hex           | shape error, fails the check                                           |
| frontmatter mapping/list/scalar parsing       | correct values; nested YAML rejected                                   |
| widened lint walk covers `docs/core/`         | a violation in any `docs/` subdirectory is reported                    |
| lint rule 1 accepts list items                | `sources:` list produces no diagnostic                                 |
| lint rule 1 still rejects nested YAML         | `regenerated_from:` with a nested list errors                          |
| `status: proposed` / `superseded` / `partial` | accepted                                                               |
| `status: bogus`                               | rejected                                                               |
| lint finds nothing wrong                      | 0 diagnostics on the live corpus after the rule repairs                |

`docs_audit.rs` currently holds one `#[test]`; the new cases are the first real coverage
that module has had.

## Migration and Sequencing

Committed in two steps, with the gate live in the first, as approved:

1. **Commit 1** — `frontmatter.rs`; replace `check_freshness` / `parse_watches` /
   `FreshnessResult` in `docs_audit.rs`; `--stale` in `main.rs` and
   `xtask/schema/cli.schema.json`; `enforce_staleness` in `verify`; widen `lint_docs` to
   `docs/` with both rule repairs; delete `source_sha` from all 38 stamped docs;
   `doc-audit.nu` delegation; `SKILL.md`, `AGENTS.md`, `DOCS_AUDIT.mbx.md`,
   `XTASK_CLI.mbx.md` corrections; `taskit protocol drift --update`.
   **This commit's `verify` run is red by construction**, because 18 living docs are
   stale at the moment the gate turns on. `pre-commit` is green within the commit once the
   rule repairs and the scalar deletion land together — the lint guard and the migration
   are not separable.
2. **Commit 2** — regenerate the 18 living docs, turning `verify` green.

The 18 living docs: 14 under `docs/core/*.mbx.md`, plus `USAGE.md`, `TESTING.md`,
`DEVELOPMENT.md`, and `.claude/skills/docs/SKILL.md`. Regeneration means re-deriving
content from current code — a stamp-only touch cannot clear a finding, by design.

Measured under the derived rule: 35 stale, 3 fresh, split 18 living and 17 historical.
That count is a floor: the measurement used raw `git log -1` for the doc, and excluding
stamp-only commits can only move a doc's substantive change earlier and add stale docs.

The 5 stamped files under `.ctx/_WORKING_DIR/` are gitignored scratch and are not
migrated. The four `.worktrees/` checkouts carry their own copies and are migrated when
their branches land.

## Out of Scope

- Backfilling `regenerated_from` on any doc.
- The 89 unstamped docs, including `docs/src/**` mdbook sources. A doc with no `sources:`
  key is not collected and is not a finding.
- The `generated:` field.
- Rewriting `docs/designs/**` or `docs/plans/**` content.
- Linting top-level `*.md`. `USAGE.md`, `TESTING.md`, `DEVELOPMENT.md`, and
  `.claude/skills/docs/SKILL.md` fall outside a `docs/` walk; extending the walk to the
  repository root is a separate decision.
- Normalising the 11 non-canonical `status:` values. The allowlist is widened instead.
- The `#528` dirty-tree symptom, which `restamp_staged_docs` already handles; issue `#528`
  as written should be corrected or closed separately once this lands.
- Any change to the `Last updated:` stamp behaviour.
- The C6 leaky `taskit-engine` test, which is a separate investigation.

## Risk

- [x] **Breaking API changes: no.** `xtask` is `publish = false`, bin-only, with no lib
      target and no external consumers. Changes are crate-internal.
- [x] **Serialization format change: yes, contained.** `AuditReport.freshness` changes
      element type; `xtask/docs-audit-report.json` is rewritten on `--full`. No code
      consumer exists; three documentation references need updating.
- [x] **New external dependency: no.** A YAML crate was considered and rejected; the
      strict-subset line parser avoids it and matches existing `docs_lint` posture. The
      cost is that nested YAML is a shape error rather than parsed.
- [x] **Protocol surface change: yes.** `xtask/src/docs_audit.rs` is hash-tracked in
      `taskit.toml`, so `taskit protocol drift --update` must run in the same change or
      the drift gate breaks.
- [x] **Feature flag required: no.**
- [x] **Gate lands red by construction.** 18 living docs are stale when the gate turns on;
      accepted deliberately, resolved by commit 2.
- [x] **Widening the lint walk changes `pre-commit` behaviour for 120 files.** It was
      previously a 0-file no-op, so `pre-commit` will start reporting for the first time.
      Measured exposure is 467 rule-1 and 11 rule-2 violations, all repaired at the rule
      level in the same commit; the remaining risk is a frontmatter shape not present in
      the corpus today, which surfaces as a new diagnostic rather than a silent pass.
- [x] **Concurrent writers.** The working tree currently carries unrelated `.ctx/`
      modifications and four live worktrees. `cargo fmt --all` would rewrite another
      writer's Rust; use `cargo fmt --all --check`, and stage explicit paths.
