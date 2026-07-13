# Active context

**Current focus (2026-07-07):**

`develop` branch has 5+ unpushed commits. Recent work spans three areas:

1. **Benchmark crate** (`crates/minibox-bench`) — dedicated bench crate with 8
   Criterion targets (layer_extract, image_pull, linux_rootfs, cgroup, spawn, etc.),
   Justfile, nightly CI job (prefers self-hosted runner, falls back to GH-hosted),
   `cargo xtask bench --check` for per-env baseline regression. Needs
   `ACTIONS_RUNNER_READ_TOKEN` secret for runner-preference logic.

2. **MoA review HIGH fixes** — waves 1+2 resolved F1-F8/D2 findings; workspace
   bumped to v0.31.0; `crates/ail` and `crates/minibox-bench` registered as
   workspace members (13 total).

3. **Structured errors** — miette diagnostics added for rich CLI error rendering
   (cf37b05a); plan doc at `docs/plans/2026-07-07-structured-errors-miette.md`.

4. **Conformance macro** — `conformance_test!` macro replaces boilerplate in
   minibox-testsuite (4ce6ce9f); design doc at
   `docs/designs/2026-07-01-conformance-macro-design.md`.

**In progress:**

- [ ] test-in-vm xtask (P1) — dual backend minibox+smolvm; pull tests pass,
      overlay/cgroup blocked by smolvm CAP_SYS_ADMIN restriction
- [ ] macOS exec/logs via VM adapters — run+stdout streaming works,
      exec-into-running unsupported
- [ ] Merge develop -> next (pending CI green on develop)

**Recently completed:**

- [x] MoA review HIGH fixes (waves 1+2) — 54510f59, 8b842b53
- [x] minibox-bench crate (waves A-C) — b9df139f, 2dd9d4ca, 1eee8706
- [x] conformance_test! macro — 4ce6ce9f
- [x] miette diagnostics for CLI — cf37b05a
- [x] Mistakes ledger — `.ctx/memory-bank/mistakes.md` (30 patterns)
- [x] crux pipeline integration — a2f04782..a913d4ea
- [x] PR-based auto-promote cascade CI — c1a16d8e

**Decisions (recent):**

- All Python removed — scripts use Rust (rust-script) or Nushell
- VZ.framework adapter removed (2026-05-08, Apple ARM64 bug)
- smolvm is default macOS adapter, krun is fallback
- Stabilization freeze active
- smolbox crate houses smolvm + krun adapter implementations

**Open questions:**

- fixture-consolidation (#355) blocked — duplicate test_fixtures.rs
- CI coverage gaps for property tests, borrow fixtures, sandbox tests
- `ACTIONS_RUNNER_READ_TOKEN` secret not yet set in GHA — bench runner preference inert

_Update when the task or branch focus changes._
