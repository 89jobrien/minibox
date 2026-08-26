# Progress

## What works

### Core container lifecycle (Linux native adapter)

- **Pull**:
    - OCI image pull from Docker Hub v2 + ghcr.io,
    - anonymous auth,
    - parallel layer downloads,
    - content-addressed caching
- **Run**:
    - Full namespace isolation (PID, NET, UTS, IPC, MNT),
    - cgroups v2 resource limits (memory, CPU weight, PID limit),
    - overlay filesystem layering
- **Exec**:
    - setns-based exec with PTY support (`-it`),
    - interactive terminal
- **Stop/rm**:
    - Graceful shutdown with signal forwarding,
    - cleanup of cgroups + overlay mounts
- **ps**:
    - List running/stopped containers with status, image, command, created
- **logs**:
    - Stored stdout/stderr capture per container
- **events**:
    - `minibox events` streams lifecycle events via event bus
- **pause/resume**:
    - cgroup.freeze-based pause/resume
- **Bind mounts**:
    - `-v`/`--mount` with host path validation
- **Privileged mode**:
    - `--privileged` flag
- **Bridge networking**:
    - veth pairs,
    - NAT via iptables DNAT
    - `MINIBOX_NETWORK_MODE=bridge`
- **Image management**:
    - prune / rmi with lease-based ImageGarbageCollector
- **Image build pipeline**:
    - push,
    - commit,
    - build (experimental)
- **Port forwarding**:
    - DNAT via iptables in bridge network mode
    - `BridgeNetwork::apply_port_mappings`
    - teardown on cleanup
- **DNS config**:
    - defaults to 8.8.8.8 + 8.8.4.4,
    - configurable per container
- **Execution manifest**:
    - persisted before spawn,
    - workload digest,
    - env value
    - SHA-256 (never plaintext),
    - policy evaluation via `mbx verify`
- **OTEL trace export**:
    - full OTLP/gRPC exporter via opentelemetry 0.31,
    - batch export, graceful fallback,
    - OtelGuard shutdown
    - Wired in miniboxd main.
- **Auth-policy gate**:
    - `ContainerPolicy` deny-by-default for bind mounts and privileged mode
    - Env-var opt-in (`MINIBOX_ALLOW_BIND_MOUNTS`, `MINIBOX_ALLOW_PRIVILEGED`)
    - `validate_policy()` enforced on every run request

### GKE adapter

- Unprivileged pod execution via proot + copy-FS
- Image pull/push support via OciPushAdapter

### macOS adapters

- **smolvm** (default): run/stop/ps via lightweight Linux VM
- **krun**: run/stop/ps via libkrun VM (fallback when smolvm absent)
- **colima**: run/stop/ps via Lima VM (Intel Macs); `ColimaContainerCommitter` added for
  commit/save (nerdctl commit/save -> docker-archive import) (1ae7528e)
- Limitations: exec/logs not supported on any macOS adapter

### Testing infrastructure

- ~1,467 tests total across all categories
- ~728 inline unit tests + ~739 integration test files
- 19 security regression tests pinning all 12 invariants
- ~46 proptest property tests (protocol roundtrip, cgroup bounds, daemon state)
- 28 conformance tests (backend-agnostic adapter trait contracts)
- 11 borrow-reasoning fixtures (must-pass/must-fail)
- 15 e2e daemon+CLI tests (Linux+root)
- 16 cgroup integration tests (Linux+root)
- ~17 sandbox tests, 30 CLI subprocess tests
- Conformance suite generates Markdown/JSON reports
- e2e showcase suite (`crates/minibox-testsuite/src/showcase/`) — narrated demo
  scenarios (e.g. lifecycle: run/stop/rm), backs a CLI demo mode; xtask CLI schema
  at `xtask/schema/cli.schema.json` (02194fd9, 3b9b85bd)

### CI pipeline

- 8 GHA workflows:
    - lint,
    - test,
    - conformance,
    - protocol drift,
    - nightly audit,
    - release
- Self-hosted runner on VPS for Linux-specific tests
- Pre-commit/pre-push local gates via cargo xtask

### Developer tooling

- `mbx doctor` — preflight diagnostics (compiled adapters, capabilities)
- `mcp` / `minibox-mcp` — MCP stdio server exposing agent-safe daemon tools
  for doctor, ps, images, logs, manifest, pull, run, stop, and rm
- `mbx tui` — new read-only `minibox-tui` crate (ratatui + crossterm): live container table
  (polls `DaemonRequest::List` every 1s) and live-tailing lifecycle event log
  (`DaemonRequest::SubscribeEvents`); no run/stop/exec by design (adf70510)
- `mbx completions` — hidden subcommand generating a Nushell completion script via
  `clap_complete`/`clap_complete_nushell`, sourced through `nu_libs.nu` (b9a84847)
- `cargo xtask musl-check` — new prepush gate catching `cfg(target_os = "linux")` build
  failures against the musl target before CI (1ae7528e)
- `cargo xtask ci-watch` — watch GHA run status with job-level detail
- `nu scripts/promote.nu` — branch cascade (develop->next->staging->main)

## Recently completed
- **Protocol drift expectation fix** — xtask's expected surface registry updated to track the
  already-split `domain-*` entries instead of the stale single `domain-ports` entry; file-level
  clippy allow for unwrap/expect/panic in `cli_subprocess.rs` (fe9bae3e).
- **Colima commit adapter + image lease conformance** — `ColimaContainerCommitter`
  (nerdctl commit/save -> docker-archive import), `ImageLeaseService` port conformance suite +
  `InMemoryLeaseService` test double, `ContainerRecord.upper_dir`/`merged_dir` populated from
  rootfs metadata, `xtask musl-check` prepush gate (1ae7528e).
- **minibox-tui crate** — new read-only TUI dashboard crate, `mbx tui` subcommand, 6 unit tests
  covering App state transitions, live-smoke-tested against `miniboxd` (adf70510).
- **Nushell completion generation** — `clap_complete_nushell`-backed hidden `completions`
  invocation intercepted before clap parsing (b9a84847).
- **MCP control surface first slice** — new `crates/mcp` workspace package
  `minibox-mcp`, with Rust library and binary names set to `mcp`. Exposes
  typed stdio MCP tools backed by `minibox-core::client::DaemonClient`,
  policy-gated mutating/high-risk options, miette diagnostics, and unit +
  stdio integration tests.

- **smolvm async/sync boundary fix** — `SmolVmRegistry`/`SmolVmRuntime::vm_exec` and the
  spawn_process command path ran `std::process::Command::output()` inline in async fns with
  no `spawn_blocking`, starving the tokio worker on long-running VM ops (boot+pull+workload
  can exceed a minute). Fixed and confirmed via live repro: `mbx ps` stays responsive during
  a backgrounded long-running container (94f227b9).
- **mbx pause/resume + ps polling fixes** — corrected terminal response handling for
  pause/resume and the ps polling parser (e5c40152).
- **e2e showcase suite** — narrated demo scenarios in
  `crates/minibox-testsuite/src/showcase/`, xtask CLI schema
  (`xtask/schema/cli.schema.json`); lifecycle scenario fixed to expect `rm` after `stop`
  on an ephemeral run (02194fd9, 3b9b85bd).
- **Docs audit fixes** — crate count, doc links, stale version refs, domain.rs
  attribution, path prefixes, test file counts corrected (10a03d62, d9121ac3).
- **minibox-bench crate** — dedicated Criterion benchmark crate with 8 hot-path
  targets (layer_extract, image_pull, linux_rootfs, cgroup, spawn), Justfile,
  `cargo xtask bench --check` regression gate, nightly CI job with self-hosted
  runner preference (b9df139f, 2dd9d4ca, 1eee8706, 5fa390cb).
- **MoA review HIGH fixes** — resolved F1-F8/D2 findings across two waves;
  workspace bumped to v0.31.0; ail + minibox-bench crates registered (54510f59,
  8b842b53).
- **conformance_test! macro** — replaces ConformanceTest boilerplate in
  minibox-testsuite; design doc at docs/designs/2026-07-01-conformance-macro-design.md
  (4ce6ce9f).
- **miette diagnostics** — rich CLI error rendering via miette; plan doc at
  docs/plans/2026-07-07-structured-errors-miette.md (cf37b05a).
- **PR-based auto-promote CI** — cascade develop->next->staging->main via PR
  workflow (c1a16d8e).
- **Open PR merge pass + final verification** — #462, #460, #459, #464, and
  #324 merged; open PR list empty. `cargo xtask verify` passed for task `t12`
  on 2026-07-26, but the verify checkpoint was not recorded because the
  checkout remains dirty with pre-existing work.
- **Workspace-wide clippy sweep** — 64 Linux-only warnings resolved (826a54ed).
- **Rustqual SRP sweep** — workspace-wide SRP_PARAMS/FRAGMENT/BOILERPLATE/TQ
  elimination (8212a9a4).
- **Mistakes ledger** — .ctx/memory-bank/mistakes.md (30 recurring patterns).
- **Python code removal** — removed all 15 Python files + uv.lock from scripts/.
  ai-review.nu now calls ai-review.rs (rust-script). Gitea CI diagnose job
  removed. No Python remains in the project.
- **Dead script cleanup** — removed gen-class-diagrams.py, install-claude-skills.sh,
  demo-smolvm.sh, agent_hello_world.py (no callers found).
- **14-doc audit and fix** — verified all 20 docs/core/ files against code.
  Fixed 19 critical errors and 10 stale references across 14 docs:
  smolbox crate visibility, minibox-conformance->minibox-testsuite rename,
  version 0.24.0->0.30.0, vz adapter removal, DoS limit corrections,
  select_adapter->adapter_from_env, stable->staging, minibox-cli->mbx,
  SO_PEERCRED/handler file attributions.
- **Rustqual SRP sweep** — RunParams extraction, named constants in miniboxd,
  dead code removal, duplicate dedup (lima_exec, lifecycle handlers),
  long function extraction across core+minibox+miniboxd (791e81a..dd3e9a6)
- **test-in-vm xtask** — dual backend minibox+smolvm, pull tests pass (41279b1)
- **smolvm output_reader fix** — fixed adapter output streaming (747b636)
- **CI fixes** — InternalPath/PathBuf mismatches, hyper pin, RunParams refactor
  fallout (c7a82bf, 64beade, 0c8e693, b930406, a179c64)
- **TODO markers** — GitHub issue refs from code review (b54d05d)
- **Exhaustive small-domain tests** — path validation edge cases (870aa9f)
- **Protocol/domain roundtrip property tests** — proptest coverage (6277bde)
- **Test helper cleanup** — fixes #403-#409 (f8f677a)

## In progress

- macOS exec/logs via VM adapters — container run + stdout streaming works
  (smolvm/krun). `exec_runtime: None` on both means exec-into-running is
  unsupported. No historical log retrieval.
- Merge develop -> next (pending CI green on develop)

## Not started / backlog

### Partially done

- **Container-in-container** (DinD):
    - UX — works today via privileged mode + bind mounts (full e2e test in system_tests.rs). Missing: turnkey `--dind`
      flag, auto-provisioned inner daemon, socket forwarding, shared image cache
- **Capability dropping**:
    - Grant path implemented (`apply_full_capabilities` with curated whitelist minus SYS*MODULE/SYS_BOOT/MAC*\*)
    - Drop path for unprivileged containers missing (no default allowlist like Docker's ~14 caps)
- **User namespace remapping**:
    - `RuntimeCapabilities.supports_user_namespaces` field exists on every adapter
    - no `CLONE_NEWUSER`
    - no uid_map/gid_map writing
    - no newuidmap helper
    - purely informational today
- **Windows (WSL2)** adapter:
    - ~40% scaffolded. `Wsl2Runtime`/`Wsl2Filesystem`/`Wsl2Limiter` implement traits via `wsl.exe` delegation
    - `winbox` crate has module stubs + conformance test
    - Not wired into miniboxd
    - no Named Pipe server
    - no `minibox-wsl-helper` binary exists

### Not started

- **Seccomp filters**:
    - zero code
    - no BPF profiles
    - no syscall filtering
- **Rootless support**:
    - blocked on user namespace impl
    - no slirp4netns
    - no fuse-overlayfs
    - no unprivileged cgroup delegation
- **CRI compliance**:
    - zero protobuf/gRPC
    - no RuntimeService/ImageService
- **Aggregate image size limit**:
    - per-layer 10 GiB enforced
    - no total budget across layers in a single pull (Issue #319)
- **ValidatedPath newtype**:
    - all validation is function-call based (`validate_layer_path()`)
    - no type-level guarantee
    - documented in SECURITY_INVARIANTS.md as a consideration

## Known issues

- Stabilization freeze (#127) closed 2026-08-22 — 5/6 acceptance sub-issues done
  (#122 protocol, #114 state, #120 docs, #123 security, #117 support tiers, #133
  CI enforcement); #116 (handler.rs coverage) remains open and tracked separately
- krun metrics inconsistency — constructs its own `NoOpMetricsRecorder`
  internally rather than accepting the shared broker
- Colima path validation bypass via `..` (P1)
- Bind-mount teardown absent on container cleanup (P2)
- `xtask` binary not currently available (preflight reports fail)
- Mock system duplication — `crates/minibox/src/adapters/mocks.rs` is a ~1000-line
  near-total reimplementation of `crates/minibox-core/src/adapters/mocks.rs` for the
  same 5 mock types (~62 duplicate occurrences via `dupehound scan`). Fix candidate:
  replace with `pub use minibox_core::adapters::mocks::{...}` re-export, consistent
  with `minibox`'s existing re-export-of-`minibox-core` convention. Filed as task t23.
- CI coverage gaps — property tests, borrow fixtures, sandbox tests, CLI
  subprocess tests, krun conformance not in any CI workflow
- macOS VZ.framework — blocked by Apple bug on ARM64; adapter removed 2026-05-08
