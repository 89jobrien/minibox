# Active context

**Current focus (2026-07-25):**

`develop` branch. Recent work spans:

1. **smolvm async/sync boundary fix** (94f227b9) — `SmolVmRegistry`/`SmolVmRuntime::vm_exec`
   and the spawn_process command path called `std::process::Command::output()` inline inside
   async fns with no `spawn_blocking`, violating the repo's async/sync boundary rule and
   starving the tokio worker for the container's request handling during long-running VM
   invocations (boot + pull + workload can exceed a minute). Fixed + live-repro confirmed
   (`mbx ps` now stays responsive during a backgrounded long-running container).

2. **mbx pause/resume + ps polling fixes** (e5c40152) — terminal response handling for
   pause/resume, ps polling parser corrected.

3. **e2e showcase suite** (02194fd9, 3b9b85bd) — new end-to-end showcase suite
   (`crates/minibox-testsuite/src/showcase/`), narrated demo, xtask CLI schema
   (`xtask/schema/cli.schema.json`). Lifecycle scenario fix: expected `rm` after `stop`
   on an ephemeral run.

4. **Docs audit fixes** (10a03d62, d9121ac3) — crate count, doc links, stale version refs,
   domain.rs attribution, path prefixes, test file counts corrected across docs/core/.
5. **MCP control surface first slice** — new `minibox-mcp` package at `crates/mcp`,
   publishing the `mcp` library and binary names. Stdio MCP server wraps existing
   daemon protocol operations for doctor, ps, images, logs, manifest, pull, run,
   stop, and rm with agent-specific policy gates and miette diagnostics.

6. **Prior session (2026-07-07 and earlier)**: benchmark crate (`crates/minibox-bench`,
   8 Criterion targets), MoA review HIGH fixes (v0.31.0, 13 workspace crates), miette
   diagnostics for CLI errors, `conformance_test!` macro in minibox-testsuite.

**In progress:**

- [ ] test-in-vm xtask (P1) — dual backend minibox+smolvm; pull tests pass,
      overlay/cgroup blocked by smolvm CAP_SYS_ADMIN restriction
- [ ] macOS exec/logs via VM adapters — run+stdout streaming works,
      exec-into-running unsupported
- [ ] Merge develop -> next (pending CI green on develop)

**Recently completed:**

- [x] smolvm spawn_blocking fix for vm_exec/spawn_process — 94f227b9
- [x] MCP control surface first slice — `crates/mcp` / package `minibox-mcp`
- [x] mbx pause/resume + ps polling fixes — e5c40152
- [x] e2e showcase suite, narrated demo, xtask CLI schema — 02194fd9, 3b9b85bd
- [x] docs audit (crate count, links, version refs, attribution) — 10a03d62, d9121ac3
- [x] MoA review HIGH fixes (waves 1+2) — 54510f59, 8b842b53
- [x] minibox-bench crate (waves A-C) — b9df139f, 2dd9d4ca, 1eee8706
- [x] conformance_test! macro — 4ce6ce9f
- [x] miette diagnostics for CLI — cf37b05a
- [x] Mistakes ledger — `.ctx/memory-bank/mistakes.md` (30 patterns)
- [x] crux pipeline integration — a2f04782..a913d4ea
- [x] PR-based auto-promote cascade CI — c1a16d8e
- [x] Open PR merge pass — #462, #460, #459, #464, and #324 merged; open PR list empty on 2026-07-26
- [x] Final verification task `t12` — `cargo xtask verify` passed; verify checkpoint not recorded because checkout remains dirty with pre-existing work

**Decisions (recent):**

- All Python removed — scripts use Rust (rust-script) or Nushell
- VZ.framework adapter removed (2026-05-08, Apple ARM64 bug)
- smolvm is default macOS adapter, krun is fallback
- Stabilization freeze active
- smolbox crate houses smolvm + krun adapter implementations
- Blocking subprocess calls in async fns must always route through
  `tokio::task::spawn_blocking`, even for adapter/VM exec paths that "look" fast —
  smolvm CLI can block over a minute (94f227b9 root cause)

**Open questions:**

- fixture-consolidation (#355) blocked — duplicate test_fixtures.rs
- CI coverage gaps for property tests, borrow fixtures, sandbox tests
- `ACTIONS_RUNNER_READ_TOKEN` secret not yet set in GHA — bench runner preference inert

_Update when the task or branch focus changes._
