# Active context

**Current focus:**

MoA review HIGH fixes and benchmark redesign landed on `develop` (2026-07-07,
5 unpushed commits). Workspace at v0.31.0, 13 members: crates/ail and
crates/minibox-bench registered. minibox-bench owns all benchmarks (8 targets
incl. layer_extract, image_pull, linux_rootfs/cgroup/spawn); `cargo xtask
bench --check` does per-env baseline regression checking; nightly CI bench
job prefers the self-hosted runner and falls back to GH-hosted (needs
ACTIONS_RUNNER_READ_TOKEN secret for the preference to work).

**In progress:**

- [ ] test-in-vm xtask (P1) — dual backend minibox+smolvm; pull tests pass,
      overlay/cgroup blocked by smolvm CAP_SYS_ADMIN restriction
- [ ] macOS exec/logs via VM adapters — run+stdout streaming works,
      exec-into-running unsupported
- [ ] Merge develop -> next (pending CI green on develop)
- [x] crux pipeline integration — promote, verify, pre-commit, prepush, merge
      pipelines wired (a2f04782..a913d4ea)
- [x] Mistakes ledger created — `.ctx/memory-bank/mistakes.md` catalogs 30
      recurring patterns across xtask, CI, reverts, handler errors

**Decisions (recent):**

- All Python removed from project — scripts use Rust (rust-script) or Nushell
- VZ.framework adapter removed (2026-05-08, Apple ARM64 bug)
- smolvm is default macOS adapter, krun is fallback
- Stabilization freeze active
- smolbox crate houses smolvm + krun adapter implementations (not macbox)

**Open questions:**

- fixture-consolidation (#355) blocked — duplicate test_fixtures.rs
- CI coverage gaps for property tests, borrow fixtures, sandbox tests

_Update when the task or branch focus changes._
