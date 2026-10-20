---
source_sha: 583d795681594db0435e7fbc4d196e19edfef358
sources:
  - Cargo.toml
  - crates/minibox-domain
  - crates/minibox-core
  - crates/minibox
  - crates/minibox-macros
  - crates/miniboxd
  - crates/macbox
  - crates/smolbox
  - crates/winbox
  - crates/mbx
  - crates/minibox-crux-plugin
  - crates/mcp
  - crates/minibox-testsuite
  - crates/minibox-bench
  - crates/ail
  - crates/minibox-cni
  - crates/minibox-tui
  - crates/minibox-signpost
  - xtask
generated: 2026-09-16
---

# Crate Inventory

## Summary

| Crate               | Type       | LOC    | Source files | Test files              | Features                  |
| ------------------- | ---------- | ------ | ------------ | ----------------------- | ------------------------- |
| minibox-domain      | lib        | ~6.1k  | 21           | inline                  | test-utils                |
| minibox-core        | lib        | ~12.6k | 28           | 10 integration + inline | test-utils, fuzzing       |
| minibox             | lib        | ~21.5k | 71           | 56 integration + inline | test-utils, metrics, otel |
| minibox-macros      | lib        | ~175   | 9            | 1 integration           | --                        |
| miniboxd            | bin+lib    | ~1.6k  | 4            | 13 integration + inline | metrics, otel, tailnet    |
| macbox              | lib        | ~3.6k  | 16           | 5                       | --                        |
| smolbox             | lib        | ~148   | 4            | 2 integration           | --                        |
| winbox              | lib        | ~280   | 5            | 1 integration           | --                        |
| minibox-cli         | bin        | ~3.2k  | 18           | 3 integration + inline  | subprocess-tests, tui     |
| minibox-crux-plugin | bin        | ~1.2k  | 2            | 1 integration           | --                        |
| minibox-mcp         | lib+bin    | ~1.6k  | 11           | 1 integration           | --                        |
| minibox-testsuite   | lib+bin    | ~3.7k  | 27           | inline                  | --                        |
| minibox-bench       | lib        | ~1.4k  | 4 + 8 benches | inline fixture tests   | --                        |
| ail                 | bin        | ~4     | 1            | 0                       | --                        |
| minibox-cni         | lib+2 bins | —      | —            | integration + inline    | --                        |
| minibox-tui         | lib        | —      | —            | inline                  | --                        |
| minibox-signpost    | lib        | ~200   | 1            | inline                  | macos-signposts           |
| xtask               | bin        | ~5k    | 35           | 0                       | --                        |

**Estimated total:** run `cargo xtask info metrics` for the current workspace
member count and Rust source-line total. All crates are at version 0.34.1
(xtask 0.1.0).

<!-- fact:workspace_version=0.34.1 -->

---

## minibox-domain

Pure inner dependency ring and canonical owner of domain values, policies,
lifecycle events, and ports. Production dependencies contain no async runtime,
HTTP client, process runner, socket transport, or filesystem adapter.

**Key modules:** `runtime.rs`, `filesystem.rs`, `image.rs`, `exec.rs`,
`networking.rs`, `events.rs`, `execution_manifest.rs`, `execution_policy.rs`,
`workflow.rs`, and typed ID/path values.

---

## minibox-core

Cross-platform shared infrastructure. Single source of truth for protocol,
shared errors, image management, registry clients, and the Unix socket client;
`domain/` is a compatibility facade over `minibox-domain`, the canonical owner of domain values,
ports, policies, and lifecycle events.

**Key modules:** `protocol.rs` (DaemonRequest /
DaemonResponse), `domain/` (compatibility re-exports), `image/` (ImageStore,
RegistryClient, layer
extraction, GC, leases, Dockerfile parser), `client/` (DaemonClient,
DaemonResponseStream), `events.rs` (event-broker adapter and compatibility exports),
`adapters/` (HostnameRegistryRouter, mocks,
test_fixtures, conformance).

**External deps:** serde, tokio, reqwest, anyhow, thiserror, tracing, sha2,
tar, flate2, slashcrux (Priority/Urgency/ExecutionContext for RunPipeline/Run).

---

## minibox

Largest crate. Linux container primitives + all platform adapter
implementations + daemon server/handler/state + testing infrastructure.

**Key modules:**

- `lib.rs` (compatibility re-export of the `minibox-core::domain` facade)
- `container/` (Linux only): namespace.rs, cgroups.rs, filesystem.rs,
  process.rs
- `adapters/`: native (overlay, cgroup, namespace, bridge network), gke
  (copy FS, proot, noop limiter), colima (lima/nerdctl), and shared stubs
  (vf, hcs, wsl2, docker_desktop), mocks
- `daemon/`: handler/ (HandlerDependencies, request routing), server.rs
  (Unix socket listener, SO_PEERCRED auth), state.rs (DaemonState),
  telemetry/ (module directory: `mod.rs`, `noop.rs`, `traces.rs`,
  `prometheus_adapter.rs`, `server.rs`), network_lifecycle.rs
- `image/` (re-exported from minibox-core)
- `testing/`: mocks/, fixtures/, helpers/, backend/, capability

**Features:** `test-utils` (mocks + fixtures + conformance), `metrics`
(Prometheus endpoint), `otel` (OTLP trace export).

**Benchmarks:** none — all criterion benches live in `crates/minibox-bench`.

---

## minibox-macros

Declarative `macro_rules!` macros for adapter boilerplate reduction.

**Macros:** `as_any!` (downcasting), `default_new!` (Default via new()),
`adapt!` (both), `provide!` (LLM provider constructors), `require_capability!`
(test gating), `normalize_name!`/`normalize_digest!`/`normalize!`/
`denormalize_digest!` (path normalization), `test_run!` (test DaemonRequest
builder).

---

## miniboxd

Daemon binary. Platform-dispatches: Unix (Linux + macOS) -> `run_daemon()`,
Windows -> `winbox::start()`.

**Key modules:** `adapter_registry.rs` (re-export shim only —
`pub use minibox_core::adapter_registry::*`; the `AdapterSuite` enum and
`adapter_from_env` live in `crates/minibox-core/src/adapter_registry.rs`),
`listener.rs` (UnixServerListener), `config.rs`.

**Adapter suites:** native, gke, colima, smolvm (default), krun (fallback), plus the
feature-gated VZ startup path (currently blocked during VM boot).

---

## macbox

macOS platform support. Krun implementations live in `macbox::krun`; Colima
composition and the feature-gated VZ path also live here.

**Backends:**

- **Colima**: `ColimaRegistry`, `ColimaRuntime`, `ColimaFilesystem`,
  `ColimaLimiter` -- delegates to `colima ssh`/limactl/nerdctl
- **krun**: `KrunRegistry`, `KrunRuntime`, `KrunFilesystem`, `KrunLimiter` --
  libkrun micro-VMs (HVF on macOS, KVM on Linux)

---

## smolbox

Compatibility facade for smolvm and krun adapter imports.

**Backends:**

- **smolvm** re-exports implementations owned by `minibox::adapters`.
- **krun** re-exports implementations owned by `macbox::krun`.

---

## winbox

Phase 1 Windows stub. `start()` returns error unconditionally.

**Modules:** `hcs.rs` (stub), `wsl2.rs` (stub), `paths.rs` (Named Pipe
path), `preflight.rs` (detection stubs).

---

## minibox-cli (`mbx`)

CLI client. Connects to daemon via Unix socket, sends JSON requests, streams
responses.

**Subcommands:** run, ps, stop, pause, resume, rm, pull, push, exec, logs,
events, images, search, build, commit, prune, rmi, sandbox, snapshot
(save/restore/list), pipeline (run/list/show), load, doctor, capabilities,
monitor, manifest, verify, diagnose, update, upgrade, and feature-gated `tui`.
`completions` is handled before clap parsing and so does not appear in `mbx --help`.

---

## minibox-crux-plugin

Crux plugin binary. Exposes minibox container operations (pull, run, ps, stop,
rm, pause, resume, image-ls, image-rm) over JSON-RPC stdio for integration with
the crux agentic DSL runtime.

**Depends on:** minibox-core. The former `crux-plugin` dependency has been removed.

---

## minibox-mcp

MCP stdio server for agent-controlled minibox operations. Wraps existing daemon
protocol requests for doctor, ps, images, logs, manifest, pull, run, stop, and
rm, with MCP-specific policy gates around mutating and higher-risk run options.

**Binaries:** `mcp`.

**Depends on:** minibox-core, rmcp, miette.

---

## minibox-testsuite

Conformance test harness for adapter trait contracts. Not published; used
internally by `cargo xtask test conformance`.

**Binaries:** `run-conformance`, `generate-report`.

**Depends on:** minibox, minibox-core.

---

## minibox-bench

Dedicated benchmark crate. Leaf crate owning all criterion targets and fixtures; the only
place where the `test-utils` features of the lib crates are enabled. Run via `just bench` /
`just bench-check` / `just bench-baseline` (`cargo xtask bench` underneath). Results land in
`bench/results/` (gitignored); per-env baselines are tracked at
`bench/baseline.{local,selfhosted,hosted}.json`.

**Bench targets:** `protocol_codec`, `daemon_dispatch`, `trait_dispatch`, `layer_extract`,
`image_pull`, plus root-gated Linux benches `linux_rootfs`, `linux_cgroup`, `linux_spawn`.

**Fixtures:** `LayerSpec`/`build_layer_tar_gz` (deterministic OCI layers), `BenchRegistry`
(wiremock-backed OCI registry).

**Depends on:** minibox, minibox-core (both with `test-utils`), criterion.

---

## minibox-cni

CNI plugin exec protocol and chain orchestration. Ships two binaries,
`minibox-cni-bridge` and `minibox-cni-verify`. `miniboxd`'s opt-in `cni` feature
reaches `minibox_cni::CniNetworkProvider`, but the `minibox` crate still declares
`minibox-cni` without wiring it into its adapters (recorded as cargo-machete-ignored
in `crates/minibox/Cargo.toml`), so the native adapter continues to build its
bespoke `BridgeNetwork`.

---

## minibox-tui

Read-only Ratatui dashboard for container status and lifecycle events. Published as a library and
used by `minibox-cli` only when its `tui` feature is enabled.

---

## minibox-signpost

macOS `os_signpost` interval instrumentation, used by `minibox` and `macbox`.
Isolated in its own crate so that `minibox-core` keeps zero OS dependencies.
Real signpost emission is behind the non-default `macos-signposts` feature,
which pulls in Apple's Swift-bridged `apple-log` crate; `span()` is a no-op
when no profiler is attached, and `Span` is zero-sized on non-macOS targets.
Consumed by `cargo xtask profile native`.

---

## ail

Placeholder binary for the agent-improvement loop. No implementation yet.

---

## xtask

Development tool. All CI gate commands.

**Key commands:** pre-commit, prepush, verify, lint, fix, architecture, musl-check, coverage,
coverage-check, agentlint, build-test-image, setup-test-vm, test-in-vm,
test-linux, bump, promote, preflight, doctor, ci-watch, daily-orchestration,
council, bench, profile, fuzz, borrow-fixtures, nuke-test-state, clean-artifacts,
cas-add, cas-check.

**Subcommand groups:** `test <suite>` (unit, conformance, krun-conformance,
turmoil, shuttle, property, quickcheck, integration, e2e, system-suite,
sandbox, gke-profile, gke-adapter), `check <target>` (stale-names,
protocol-drift, protocol-sites, protocol-variants, adapter-coverage, no-unwrap, repo-clean),
`docs <action>` (audit, lint, update-date, sync-adapters), `info <target>` (metrics,
context, changes), `capabilities verify`.

The tree currently contains 104 tracked `.rs` files under `crates/*/tests/`; use grouped
commands such as `cargo xtask test unit` rather than the deprecated `test-unit` aliases.
