# Active context

**Current focus:**

Stabilization + quality hardening on `develop` branch. Recent rustqual
refactors (RunParams extraction, named constants, dead code removal,
long function extraction, duplicate dedup). Uncommitted work: execution
policy refactor, smolvm adapter improvements, protocol additions,
handler pipeline updates.

**In progress:**

- [ ] Uncommitted changes across 14 files — execution_policy refactor,
      smolvm adapter output_reader improvements, protocol.rs additions,
      handler run/pipeline/lifecycle updates, miniboxd main.rs DI changes
- [ ] test-in-vm xtask (P1) — dual backend minibox+smolvm; pull tests pass,
      overlay/cgroup blocked by smolvm CAP_SYS_ADMIN restriction
- [ ] macOS exec/logs via VM adapters — run+stdout streaming works,
      exec-into-running unsupported
- [ ] Merge develop -> next (pending CI green on develop)

**Decisions (recent):**

- VZ.framework adapter removed (2026-05-08, Apple ARM64 bug)
- QEMU vm_image/vm_run xtask commands removed
- smolvm is default macOS adapter, krun is fallback
- Stabilization freeze active

**Open questions:**

- fixture-consolidation (#355) blocked — duplicate test_fixtures.rs
- CI coverage gaps for property tests, borrow fixtures, sandbox tests

_Update when the task or branch focus changes._
