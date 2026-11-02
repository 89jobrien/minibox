# Release Recovery Playbook

Known release-pipeline failure modes for minibox, each with a detection signal,
an automated repair, and a command to re-verify the repair worked. The
`release` agent (see `AGENTS.md` / agent definition) consults this file before
escalating any gate failure to a human. A failure mode not listed here is
always escalated — never guessed at.

## 1. macOS asset-naming mismatch in `mbx upgrade`

**Root cause (commit `c0ba404e`, 2026-10-14):** `GitHubReleaseProvider` in
`crates/mbx/src/commands/upgrade.rs:246` built the expected asset name as
`minibox-{target_triple}.tar.gz`, but `release.yml` actually publishes
`minibox-{tag}-{target_triple}.tar.gz`. The tag segment was missing from the
expected-name format string.

- **Detection signal:** `mbx upgrade` output matches `no asset named .* in release`.
- **Automated repair:** Diff the live release's actual asset names
  (`gh release view <tag> --json assets --jq '.assets[].name'`) against the
  format string in `upgrade.rs`. If the format string is missing the `{tag}`
  segment, patch it to match the actual naming scheme and rebuild.
- **Re-verification:** `mbx upgrade --dry-run` resolves an asset successfully
  for the target triple (no "no asset named" error).

## 2. Stale binary launched from outside `CARGO_TARGET_DIR`

**Root cause (documented `active-context.mbx.md`, 2026-10-14/15):** This
workspace's `.cargo/config.toml` sets `target-dir = "/Users/joe/.minibox/cache/target"`.
A `cargo build --release` does not write to `target/release/` at the repo
root — deploying/smoke-testing from that default path silently runs a stale
binary that predates the fix under test, with no error.

- **Detection signal:** the binary about to be smoke-tested or deployed has an
  mtime older than the newest source file mtime under `crates/**/*.rs`.
- **Automated repair:** resolve the real target dir via
  `cargo metadata --format-version 1 | jq -r .target_directory` rather than
  assuming `target/release`; re-copy/re-link the binary from the resolved path.
- **Re-verification:** `stat` the binary mtime and confirm it is newer than
  every `crates/**/*.rs` mtime. For a flag/behavior change with a silent
  no-op risk, also `strings <binary> | rg <marker-unique-to-the-fix>` to
  confirm the new code path is actually present in the binary, not just that
  the build exited 0.

## 3. clippy `expect_used`/`unwrap_used` deny on test-only code

**Root cause (commit `5c1a3f8a`):** the workspace `Cargo.toml`
`[workspace.lints.clippy]` denies `unwrap_used`, `expect_used`, `panic`,
`todo`, and `unimplemented` across **all targets**, including
`#[cfg(test)] mod tests`. Newly added inline unit tests using `.unwrap()`/
`.expect()` — otherwise idiomatic in test code — trip `-D warnings`.

- **Detection signal:** clippy stderr matches
  `expect_used|unwrap_used|used_underscore_binding` on a path containing
  `/tests/` or inside a `#[cfg(test)]` module.
- **Automated repair:** add
  `#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]`
  directly on the offending `#[cfg(test)] mod tests` block — per-module, not
  workspace-wide, matching the existing convention in
  `.claude/rules/rust-patterns.md` ("Clippy Allows in Test Modules").
- **Re-verification:** `cargo clippy --workspace --all-targets -- -D warnings`.

## 4. `taskit promote` run from the wrong branch

**Root cause:** no promote step may run except from the branch it is meant to
promote _from_. Running it from `main`/`staging`/`release` by mistake can
silently promote the wrong content or skip `develop`'s unmerged commits (see
`AGENTS.md`: "Run `git branch --show-current` before every commit. If the
answer is `main`, STOP.").

- **Detection signal:** `git branch --show-current` does not equal the
  expected source tier for the requested `--from` argument before
  `cargo xtask promote --from <X> --to <Y>` is invoked.
- **Automated repair:** abort before promote runs. If the working tree is
  clean, `git checkout <expected-source-branch>` and retry the promote
  invocation. If the tree is dirty, escalate — do not discard uncommitted
  work to force a checkout.
- **Re-verification:** `git branch --show-current` matches the expected
  source branch, then the promote command is re-run.

## 5. taskit flow's hardcoded cargo CI gate (non-Rust repos)

**Root cause (confirmed):** `taskit-engine/src/health.rs:333` (in
`~/dev/taskit`) runs `cargo clippy --workspace --message-format=json`
unconditionally as part of the flow health/CI gate, with no fallback for a
repo that has no `Cargo.toml`.

- **Detection signal:** the gate fails with `cargo: command not found` (or a
  clippy error referencing a nonexistent workspace) in a repo lacking a
  `Cargo.toml` at its root.
- **Automated repair:** no in-repo fix is available — this is an upstream
  `taskit` defect, not something minibox's own tooling can patch. Skip the
  gate via `cargo xtask promote --skip-ci-check` (or the repo's equivalent
  flag) and record the skip as a facet so it surfaces for a real upstream fix.
- **Re-verification:** `taskit flow status` shows the gate as explicitly
  skipped (not hung/pending), and the promote/flow step completes.

---

**Note on facets:** no facet/evidra JSONL infrastructure existed in this repo
before this playbook. The `release` agent is the first writer of
`.ctx/releases/facets.jsonl` (one JSON object per line: `{ts, failure_mode,
detection_signal, repair_applied, reverify_result}`). Do not assume an
existing facet schema elsewhere in the workspace applies here.
