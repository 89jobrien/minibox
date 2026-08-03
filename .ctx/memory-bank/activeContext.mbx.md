# Active context

**Current focus (2026-07-31):**

`develop` branch. Most recent work (newest first, per `git log --oneline`):

1. **Docs fix pass** (9e6321ef) — README architecture diagram undercounted crates (13 -> 14,
   omitted `minibox-cni`); README/DEVELOPMENT/CLAUDE/CONTRIBUTING taught deprecated
   `cargo xtask test-unit`-style aliases instead of the canonical `cargo xtask test <suite>`
   form; `CRATE_INVENTORY.mbx.md` still referenced `handler.rs` as a single file after it
   was split into `crates/minibox/src/daemon/handler/`. Also fixed a broader batch of stale
   doc references this session: `assets/index.html` demo command used flags that don't
   parse (`--memory 64m --cpu 50` -> `--memory 67108864 --cpu-weight 50`), `CHANGELOG.md`'s
   `[Unreleased]` was 7 minor versions stale and got promoted to `[v0.31.0] - 2026-05-26`,
   `docs/core/PRERELEASE_CHANGELOG.mbx.md` had a mislabeled version header, and a batch of
   docs across `docs/core/` were missing the `crates/` prefix on source paths.

2. **minibox-cni crate landing** (a333e28b, 7b4ee0a5, 1b577da1) — new `minibox-cni` workspace
   member for CNI plugin exec protocol and chain orchestration, wired into the nextest recipe,
   BrokenPipe tolerance fix on plugin stdin writes, CNI networking OTEL design plan added.

3. **Colima/GKE adapter fixes** (d6659f56, bcd773e7) — Colima adapter privilege-drop bug,
   docker/nerdctl mismatch, container dir ownership; GKE proot adapter error messages now use
   `Display` instead of `Debug` formatting for paths, matching the tracing convention.

4. **`cargo xtask doctor` consolidation** (c7032c07) — folded `scripts/preflight.nu`'s tool,
   secret-manager auth, and smolvm checks into `cargo xtask doctor` as the single canonical
   environment-validation path; `scripts/preflight.nu` is now a lightweight SessionStart hook
   only.

5. **Mock/test infra** (677d3723, 787c5aad, b7e2653b) — `fake!` macro extracted for mock state
   locking, `install-hooks` just recipe added, protocol workflow-step terminal classification
   fix, new unit tests for `resolve_container_policy`/`AdapterSuite`/`AdapterSelectionError`.

6. **Registry error typing** (d3d1a83e) — `ManifestTooLarge`/`LayerTooLarge` `RegistryError`
   variants for clearer pull-size-limit diagnostics.

**In progress:**

- [ ] test-in-vm xtask (P1) — dual backend minibox+smolvm; pull tests pass,
      overlay/cgroup blocked by smolvm CAP_SYS_ADMIN restriction
- [ ] macOS exec/logs via VM adapters — run+stdout streaming works,
      exec-into-running unsupported
- [ ] minibox-cni wiring into miniboxd adapter suites — crate exists, CNI protocol landed,
      OTEL design plan drafted, but the daemon doesn't consume it yet
- [ ] Merge develop -> next (pending CI green on develop)

**Recently completed:**

- [x] Docs fix pass: crate count, deprecated xtask aliases, stale paths, CHANGELOG versioning — 9e6321ef and same-session predecessors
- [x] minibox-cni crate: exec protocol, chain orchestration, nextest wiring, BrokenPipe fix — a333e28b, 7b4ee0a5, 1b577da1
- [x] Colima adapter privilege-drop/docker-nerdctl/ownership fixes — d6659f56
- [x] cargo xtask doctor absorbs scripts/preflight.nu checks — c7032c07
- [x] smolvm spawn_blocking fix for vm_exec/spawn_process — 94f227b9
- [x] MCP control surface first slice — `crates/mcp` / package `minibox-mcp`
- [x] e2e showcase suite, narrated demo, xtask CLI schema — 02194fd9, 3b9b85bd
- [x] MoA review HIGH fixes (waves 1+2) — 54510f59, 8b842b53
- [x] minibox-bench crate (waves A-C) — b9df139f, 2dd9d4ca, 1eee8706
- [x] Mistakes ledger — `.ctx/memory-bank/mistakes.md` (30 patterns)

**Decisions (recent):**

- All Python removed — scripts use Rust (rust-script) or Nushell
- VZ.framework adapter removed (2026-05-08, Apple ARM64 bug)
- smolvm is default macOS adapter, krun is fallback
- Stabilization freeze active
- smolbox crate houses smolvm + krun adapter implementations
- `cargo xtask doctor` is now the single canonical preflight command (absorbed
  `scripts/preflight.nu`'s checks); the script is a lightweight SessionStart hook only
- Blocking subprocess calls in async fns must always route through
  `tokio::task::spawn_blocking`, even for adapter/VM exec paths that "look" fast —
  smolvm CLI can block over a minute (94f227b9 root cause)

**Open questions:**

- fixture-consolidation (#355) blocked — duplicate test_fixtures.rs
- CI coverage gaps for property tests, borrow fixtures, sandbox tests
- `ACTIONS_RUNNER_READ_TOKEN` secret not yet set in GHA — bench runner preference inert
- minibox-cni: crate exists and is tested but not yet wired into miniboxd's adapter suites —
  scope of that wiring work not yet defined

_Update when the task or branch focus changes._
