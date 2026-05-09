# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added

**Crate consolidation (GH #153):**

- 7-phase workspace consolidation: 13 crates reduced to 8
    - Phase 0: dropped `minibox-llm` (orphan)
    - Phase 1+2: absorbed `minibox-oci` + `minibox-client` into `minibox-core`
    - Phase 3: consolidated `daemonbox` + `linuxbox` into unified `minibox` crate
    - Phase 4: absorbed `minibox-testers` into `minibox` behind `test-utils` feature
    - Phase 5: `DEFAULT_ADAPTER_SUITE` const in `miniboxd`
- `minibox::testing` module — unified test infrastructure (mocks, fixtures, conformance)

**Adapter registry:**

- Centralized `miniboxd::adapter_registry` — typed `AdapterSuite` enum, `AdapterInfo`
  metadata, structured `AdapterSelectionError`, env-based selection via `MINIBOX_ADAPTER`
- Startup observability: logs selected adapter and available options with structured fields

**State management:**

- Container state reconciliation on daemon restart — marks stale Running containers as
  Orphaned via `ProcessChecker` trait; `KillProcessChecker` (unix-gated)
- Disk-persisted state survives daemon restarts; reconciliation runs at startup

**macOS / krun adapter:**

- krun fully wired: `KrunRuntime`, `KrunRegistry`, `KrunFilesystem`,
  `KrunLimiter` adapters wired into miniboxd; 31 conformance tests pass
- `SmolVM` adapter suite wired into `miniboxd`

**Tailnet integration:**

- `tailbox` crate — `TailnetNetwork` adapter, `TailnetConfig`, auth key resolution
  chain, gateway IP caching, per-container setup/cleanup
- `TailnetMode` enum and tailnet fields in `NetworkConfig` protocol

**Searchbox / Zoektbox:**

- `searchbox` crate — federated code search: `ZoektAdapter`, `MergedAdapter`,
  `GitRepoSource`, `FilesystemSource`, MCP stdio server, `searchboxd` binary
- `zoektbox` crate — zoekt deploy/service adapter, download+SHA256 verify

**minibox-llm:**

- Multi-turn `infer()` API, `Message`/`ContentBlock` types, Ollama auto-detection
  provider, fallback chain

**Testing:**

- Security regression suite for container-init invariants (tar traversal, symlink escape,
  path validation, socket auth)
- Handler error-path coverage raised to 80%+ (pause/resume, exec, push, commit, build)
- Conformance suite: commit/build/push, pause/resume handler, backend descriptor with
  capability flags
- Proptest expansion: all protocol variants covered
- Bridge networking unit tests
- 3 dogfood implementation plans: MCP server, sandboxed AI execution, CI agent

**Infrastructure:**

- Plugin extraction: dashbox, dockerbox, minibox-secrets, tailbox, minibox-bench moved to
  `minibox-plugins` workspace
- `cargo xtask pre-commit` — fmt-check + clippy + release build (macOS-safe)
- CI `all-features` job, coverage-check gate (handler 80% threshold)
- Stability checklist enforcement gates, protocol-drift detection workflow
- `init_tracing()` with `MINIBOX_TRACE_LEVEL` env var

### Fixed

- IPv6 panic replaced with `anyhow::bail!` in `IpAllocator`
- `KillProcessChecker` gated behind `cfg(unix)` for macOS compat
- `fork()` in exec path replaced with `nsenter` + `Command` (eliminates POSIX UB)
- Stale `minibox::`/`linuxbox::` refs cleaned up across tests and crate boundaries
- `ContainerState` unified — single source in `minibox-core`, no local duplicates
- Handler `HandlerDependencies` decomposed into ISP sub-structs
- `RegistryRouter` port replaces dual-registry fields in handler
- Conformance report writer uses `std::io::Error::other` (idiomatic)
- Clippy warnings resolved across all test and core crates

### Changed

- Workspace version bumped 0.19.0 → 0.20.0 → 0.21.0
- `linuxbox` crate renamed from `minibox` (2026-04-21), then absorbed back into `minibox`
  as unified crate (2026-04-26)
- `minibox-cli` renamed to `mbx`
- `FilesystemProvider` split into `RootfsSetup` + `ChildInit` (ISP)
- `BackendRootfsMetadata` — typed writable-layer contract for commit/build/push
- Three-tier git workflow: `main` → `next` (auto) → `stable` (manual)

## [v0.2.0] - 2026-04-14

### Added

**Container features:**

- `exec` subcommand — run commands in existing containers via `setns` + NativeExecRuntime
- Named containers — `--name` flag on `run`, name column in `ps`, `exec` by name
- Log capture and retrieval — `minibox logs <id>` subcommand; stdout/stderr stored per container
- PTY/interactive mode — `-it` flags on `run` and `exec`; raw terminal, stdin relay, SIGWINCH
  forwarding, PTY resize (`ResizePty`/`SendInput` protocol variants; `PtySessionRegistry`)
- Container events — `minibox events` command; `SubscribeEvents` protocol; lifecycle event emission
- Image GC and leases — `minibox prune`/`minibox rmi`; `ImageGc`, `DiskLeaseService` traits
- Bridge networking — `BridgeNetwork` adapter with veth pairs, NAT via iptables DNAT, IP allocator;
  enabled via `MINIBOX_NETWORK_MODE=bridge`
- Bind mounts — `-v`/`--volume`/`--mount` flags on `run`; path validation against traversal
- Privileged mode — `--privileged` flag; curated capability whitelist (not full grant)
- Docker-in-Docker — DinD integration test; Colima adapter support for nested miniboxd
- OCI image push — `OciPushAdapter` implementing OCI Distribution Spec push
- Container commit — `ContainerCommitter` trait; overlay upperdir snapshot to new image
- Image build — `ImageBuilder` trait; `DockerfileParser`; `MiniboxImageBuilder` (basic subset)
- Container freeze/thaw — cgroup-based SIGSTOP/SIGCONT (`Prune` protocol)

**Observability:**

- OpenTelemetry tracing — OTLP exporter, optional OTLP bridge; handlers instrumented with spans
- Prometheus metrics — `/metrics` HTTP endpoint; `MetricsRecorder` domain port;
  `MetricsTab` in dashbox with live poll and snapshot fallback
- Structured tracing contract — canonical `key = value` fields, message prefix convention,
  severity rules enforced across all handlers

**Dashbox TUI:**

- Initial `dashbox` binary (`crates/dashbox`) — 8-tab Ratatui TUI: Agents, Bench, Bench History,
  Git, Todos, CI, Diagrams, Metrics
- Space-leader command palette — `Space` opens overlay; all actions routed through
  `palette_actions()`; `CommandPalette` struct with render and key handling
- Mermaid diagram engine — parser, `.mmd` file sources, user-defined diagram loader
  (`~/.mbx/diagrams/`), navigable `OwnedDiagram` model
- Metrics tab — live Prometheus poll; snapshot fallback to STALE on offline
- Auto-refresh active tab after background command exits 0
- Todos tab wired to `doob handoff list` filtered by `minibox-` prefix
- CI tab open-URL action (`o` key)

**Agentbox (Go module):**

- `agentbox/` Go module with orchestration binary (`minibox-orchestrate`) and standalone
  `mbx-commit-msg` binary
- `council` agent — 5-role multi-perspective code review with synthesis
- `meta-agent` — design-spawn-synthesize workflow using Claude Agent SDK
- `commit-msg` tool agent — AI-generated conventional commit messages from staged diff
- `ClaudeSDKRunner` wrapping `claude-agent-sdk-go` (NDJSON over stdin/stdout)
- `FallbackChain` — sequential Anthropic → OpenAI → Gemini fallback
- `RetryingProvider` — exponential backoff with jitter
- `GitContextProvider` — repo context discovery for agent prompts

**Infrastructure and tooling:**

- `minibox-secrets` crate — typed credential store with validation, audit hashes, passthrough
- `minibox-llm` crate — multi-provider LLM client (Anthropic/OpenAI/Gemini), fallback chain,
  structured JSON output, HTTP timeouts, exponential backoff retry
- `minibox-client` crate — low-level Unix socket client library
- `dashbox` crate — Ratatui TUI binary (see above)
- `dockerbox` crate — HTTP-over-Unix-socket Docker API shim; `dockerboxd` binary; ID translation;
  `0o660` socket with group-access support; in-memory network/volume stubs
- `mbx-dagu` Go module — dagu workflow engine integration; SSE pipeline; env-var wiring;
  example workflow
- `minibox-macros` proc macros — `as_any!`, `default_new!`, `adapt!` for adapter boilerplate
- `cargo xtask bump` — workspace version bump with auto-bump pre-commit hook
- `cargo xtask test-conformance` — backend conformance report emission (Markdown + JSON)
- `cargo xtask test-linux` — cross-compile Linux test suite into OCI image, load + run via minibox
- `cargo xtask bench-vps` — VPS bench with optional `--commit`/`--push` (explicit opt-in)
- VPS bench safety — `sshpass -f <tmpfile>` instead of `-p <password>` to prevent credential leak
- Pre-push commit range resolver — hooks correctly identify the push range for new branches
- Gitea primary CI + GitHub mirror — 5-job pipeline (unit → property → integration+e2e → bench)
- CI agent hardening — package split, Anthropic/OpenAI/Gemini fallback, Gitea issue deduplication
  by commit SHA
- Self-hosted runner on jobrien-vm with mise toolchain management
- Dual MIT/Apache-2.0 license

**Testing:**

- Backend-agnostic conformance suite — `BackendDescriptor`, `BackendCapability`, fixture helpers;
  conformance tests for commit/build/push/network; Markdown + JSON report output
- Proptest suite — 33 property-based tests: DaemonState invariants, handler input safety, cgroup
  config bounds, protocol codec edge cases, digest verification, manifest parsing
- Sandbox tests — shell and Python scenario tests (stdout, stderr, exit codes, network isolation,
  script execution, JSON output, exception handling)
- DinD integration test — nested miniboxd pull and run inside a minibox container
- `just test-unit` / `just test-integration` / `just test-e2e` / `just doctor` task recipes
- Shared `OnceLock<Runtime>` for proptest (replaces per-case `make_rt()`)
- NetworkProvider conformance tests; Colima env-var + manifest.json regression tests
- ~300+ unit + conformance + property tests (any platform); 16 cgroup integration; 14 e2e

**Architecture:**

- `NetworkLifecycle` wrapper in daemonbox — best-effort cleanup on container teardown
- `NetworkMode` dispatch — `None`, `Host`, `Bridge` with `MINIBOX_NETWORK_MODE`
- `ExecRuntime`, `ImagePusher`, `ContainerCommitter`, `ImageBuilder` domain traits (ISP)
- `SessionId` newtype used in `SendInput`/`ResizePty` protocol variants
- `RootfsLayout` return type from overlay setup
- `HandlerDependencies` fully wired — all fields present in composition root and tests
- Three-tier git workflow — `main` → `next` (auto) → `stable` (manual dispatch)

### Fixed

- Absolute symlink rewriting — correctly rewritten relative to their own directory
- Mount namespace made private before `pivot_root`
- FD collection before close to avoid mid-iteration close in `close_extra_fds`
- Digest slicing bounds, descriptive timer names, scoped span guard
- Root directory tar entries (`.` and `./`) skipped to avoid false path escape error
- PID 0 validation and dynamic block device detection for `io.max`
- `fork()` inside active Tokio runtime gated behind `spawn_blocking` (exec path)
- Stdin relay fd exhaustion, exec registry leak, SIGWINCH reliability (#57 #58 #59)
- Colima: `docker images --filter` for `has_image`, strip `library/` prefix for nerdctl
- Colima: drop to `SUDO_USER` before invoking `limactl` when running as root
- CI: unit tests run on all branches (not just `next`/`stable`)
- CI: `rust-cache` removed from self-hosted jobs (shared `CARGO_TARGET_DIR`)
- CI: mise shims added to PATH before `rust-cache` on self-hosted runner
- Bench: nanosecond/microsecond field mismatch fixed with typed `BenchReport` structs
- Bench: zero-iteration suites stripped from committed baselines
- Bench: hostname redaction before committing bench results
- `adapt!` → `as_any!` for `BridgeNetwork` (fix compilation after macro rename)

### Changed

- Colima adapter uses `colima ssh` instead of `limactl shell` for executor/spawner
- `ExecConfig` split into pure `ExecSpec` (domain) + channel fields (adapter)
- Benchmark suites (codec + adapter microbench) merged into single bench run
- `--commit`/`--push` on `xtask bench-vps` are now explicit opt-in (no auto-push)
- `io.max` inner pull spans downgraded to `debug_span`

---

## [v0.1.0] - 2026-03-17

### Added

- Parallel OCI layer pulls — concurrent `tokio::spawn` per layer with progress tracking.
- `GhcrRegistry` adapter — ghcr.io OCI registry client with `WWW-Authenticate` challenge/response.
- `ImageRef` type — parses `[REGISTRY/]NAMESPACE/NAME[:TAG]`, routes to correct registry adapter.
- Local image store (`LocalStore`) — reads already-extracted layers without re-pulling.
- `macbox` crate — macOS daemon entry point, Colima preflight, paths, adapter wiring, `start()`.
- `winbox` crate — Windows daemon stub with `start()` and Named Pipe path helpers.
- `ServerListener` + `PeerCreds` traits in `daemonbox` — generic `run_server`/`handle_connection`.
- Platform dispatch in `miniboxd` — Linux → native handler, macOS → `macbox::start()`,
  Windows → `winbox::start()`.
- Platform-aware default socket/pipe path in `minibox-cli`.
- Architecture diagrams — crate-dependency-graph, hexagonal-architecture,
  platform-adapter-selection, container-lifecycle.
- `ContainerOutput`/`ContainerStopped` streaming protocol messages; `ephemeral` flag on run.
- `minibox run` exits with the container's exit code.
- `minibox-macros` crate — `as_any!`, `default_new!`, `adapt!` proc macros.
- Colima adapter wired into daemon; `io.max` block device detection.
- Wall-clock timing and tracing spans on image pull pipeline.
- Regression tests — absolute symlink rewriting, tar root-entry skip, macro contracts.
- `xtask` task runner with `pre-commit`, `prepush`, `test-unit`, `test-property`, `test-e2e-suite`.
- Self-hosted CI runner on jobrien-vm; Gitea primary + GitHub mirror setup.
- Benchmark tooling (`minibox-bench`) — codec and adapter suites, JSON report schema.
- Structured tracing contract — canonical fields, message prefixes, severity rules.

### Fixed

- `daemonbox` gates `nix` on `cfg(unix)`; Windows-compatible stubs for stop/wait.
- `miniboxd` no longer requires `compile_error!` Linux guard — dispatch is conditional.
- `io.max` PID 0 validation; correct absolute symlink rewriting.

---

## [v0.0.14] - 2026-03-19 (pre-release)

### Added

- `macbox` crate: macOS daemon entry point, Colima preflight, paths, adapter wiring, `start()`.
- `winbox` crate: Windows daemon stub with `start()` and Named Pipe path helpers.
- `ServerListener` + `PeerCreds` traits in `daemonbox`.
- Platform dispatch in `miniboxd`.
- Architecture diagrams.

### Changed

- `miniboxd` no longer requires a `compile_error!` Linux guard.
- `daemonbox` gates `nix` on `cfg(unix)`.

## [v0.0.13] - 2026-03-19 (pre-release)

### Added

- `GhcrRegistry` adapter.
- `ImageRef` type.
- Local image store (`LocalStore`).

## [v0.0.12] - 2026-03-18 (pre-release)

### Added

- `ContainerOutput` / `ContainerStopped` streaming protocol messages; `ephemeral` flag.
- `minibox run` exits with the container exit code.

### Changed

- Container stdout/stderr piped back to client via socket rather than inherited by daemon.

## [v0.0.11] - 2026-03-18 (pre-release)

### Added

- `daemonbox` crate extracted from `miniboxd`.
- 9 manifest unit tests.
- macOS CI job.

## [v0.0.10] - 2026-03-17 (pre-release)

### Added

- GitHub Actions workflows for CI, release, and integration testing.
- Security-critical tests for path validation and tar extraction safety.

### Fixed

- Clippy warnings blocking CI; `reqwest` switched to `rustls-tls`.

## [v0.0.9] - 2026-03-16 (pre-release)

### Added

- Preflight host capability probing.
- Justfile task runner.
- Cgroup v2 integration tests.
- Daemon+CLI e2e tests with `DaemonFixture`.
- `MINIBOX_SOCKET_PATH` env override.

### Fixed

- `SAFETY` comments on all unsafe blocks; deepened cgroup cleanup on container exit.

## [v0.0.8] - 2026-03-16 (pre-release)

### Added

- Benchmark tooling with CLI config, command runner, suites, report writers, stats, dry-run.

## [v0.0.7] - 2026-03-16 (pre-release)

### Fixed

- Cgroup subtree controller enablement before writing resource limits.
- Supervisor leaf cgroup for controller delegation.

## [v0.0.6] - 2026-03-16 (pre-release)

### Added

- Justfile with `sync`, `build`, `smoke`, `test` recipes.
- systemd unit file with cgroup delegation.
- `tmpfiles.d` config, install script, minibox.slice.

## [v0.0.4] - 2026-03-16 (pre-release)

### Added

- GKE unprivileged adapter suite (`proot`, copy-FS, no-op limiter). `MINIBOX_ADAPTER=gke`.
- `RuntimeCapabilities` struct and `capabilities()` on `ContainerRuntime`.
- In-memory container state tracking.

### Changed

- `Dyn` type aliases and structured `DomainError` variants.

## [v0.0.3] - 2026-03-16 (pre-release)

### Added

- Hexagonal architecture domain layer — `ResourceLimiter`, `FilesystemProvider`,
  `ContainerRuntime`, `ImageRegistry` traits.
- Infrastructure adapters for native Linux; cross-platform stubs.
- Mock adapters; cross-platform conformance suite.
- Colima/Lima adapter.
- Comprehensive security framework.

## [v0.0.2] - 2026-03-15 (pre-release)

### Security

- Fixed Zip Slip path traversal in tar extraction.
- `SO_PEERCRED` Unix socket authentication (root-only).
- Manifest/layer size limits; setuid/setgid stripping; device node rejection.

## [v0.0.1] - 2026-03-15 (pre-release)

### Added

- Initial Docker-like container runtime: `miniboxd` + `minibox` CLI.
- OCI image pulling from Docker Hub with anonymous token auth.
- Linux namespace isolation (PID, mount, UTS, IPC, network).
- cgroups v2 resource limits (`memory.max`, `cpu.weight`).
- Overlay filesystem — stacked read-only layers + per-container read-write upper dir.
- Container lifecycle: `pull`, `run`, `ps`, `stop`, `rm` over Unix socket JSON protocol.
- In-memory container state machine: Created → Running → Stopped.
- Background reaper task.
