# Active context

**Current focus:**

Codebase cleanup and doc accuracy on `develop` branch. All Python code
removed from scripts/. 14 docs/core/ files fixed against code truth.
Workspace at v0.30.0, 10 crates (smolbox added for smolvm/krun adapters).

**In progress:**

- [ ] test-in-vm xtask (P1) — dual backend minibox+smolvm; pull tests pass,
      overlay/cgroup blocked by smolvm CAP_SYS_ADMIN restriction
- [ ] macOS exec/logs via VM adapters — run+stdout streaming works,
      exec-into-running unsupported
- [ ] Merge develop -> next (pending CI green on develop)
- [ ] Commit and push doc audit fixes + script cleanup

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
