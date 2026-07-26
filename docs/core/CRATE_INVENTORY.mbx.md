# Crate Inventory

## Summary

| Crate               | Type       | LOC    | Source files | Test files              | Features                  |
| ------------------- | ---------- | ------ | ------------ | ----------------------- | ------------------------- |
| minibox-core        | lib        | ~12.6k | 28           | 10 integration + inline | test-utils, fuzzing       |
| minibox             | lib        | ~21.5k | 71           | 56 integration + inline | test-utils, metrics, otel |
| minibox-macros      | proc-macro | ~175   | 9            | 1 integration           | --                        |
| miniboxd            | bin+lib    | ~1.6k  | 4            | 13 integration + inline | metrics, otel, tailnet    |
| macbox              | lib        | ~3.6k  | 16           | 4                       | --                        |
| smolbox             | lib        | ~148   | 4            | 2 integration           | --                        |
| winbox              | lib        | ~280   | 5            | 1 integration           | --                        |
| mbx                 | bin        | ~3.2k  | 18           | 3 integration + inline  | subprocess-tests          |
| minibox-crux-plugin | bin        | ~1.2k  | 2            | 1 integration           | --                        |
| minibox-mcp         | lib+bin    | ~1.6k  | 11           | 1 integration           | --                        |
| minibox-testsuite   | lib+bin    | ~3.7k  | 27           | 3 integration           | --                        |
| minibox-bench       | lib        | ~1.4k  | 4 + 8 benches | inline fixture tests   | --                        |
| ail                 | bin        | ~4     | 1            | 0                       | --                        |
| xtask               | bin        | ~5k    | 35           | 0                       | --                        |

**Estimated total:** run `cargo xtask info metrics` for the current workspace
member count and Rust source-line total. All crates are at version 0.31.0
(xtask 0.1.0).

<!-- fact:workspace_version=0.31.0 -->

---

## minibox-core

Cross-platform shared types. Single source of truth for protocol, domain
traits, error types, image management, and the Unix socket client.

**Key modules:** `protocol.rs` (DaemonRequest /
DaemonResponse), `domain/` (domain ports, workflow/execution policy, runtime
capabilities), `image/` (ImageStore, ImageRef, RegistryClient, layer
extraction, GC, leases, dockerfile), `client/` (DaemonClient,
DaemonResponseStream), `events.rs` (ContainerEvent, EventSink/Source,
BroadcastEventBroker), `adapters/` (HostnameRegistryRouter, mocks,
test_fixtures, conformance).

**External deps:** serde, tokio, reqwest, anyhow, thiserror, tracing, sha2,
tar, flate2, slashcrux (Priority/Urgency/ExecutionContext for RunPipeline/Run).

---

## minibox

Largest crate. Linux container primitives + all platform adapter
implementations + daemon server/handler/state + testing infrastructure.

**Key modules:**

- `domain.rs` (compatibility re-exports of `minibox-core` domain ports)
- `container/` (Linux only): namespace.rs, cgroups.rs, filesystem.rs,
  process.rs
- `adapters/`: native (overlay, cgroup, namespace, bridge network), gke
  (copy FS, proot, noop limiter), colima (lima/nerdctl), smolvm, stubs
  (vf, hcs, wsl2, docker_desktop), mocks
- `daemon/`: handler.rs (HandlerDependencies, request routing), server.rs
  (Unix socket listener, SO_PEERCRED auth), state.rs (DaemonState),
  telemetry.rs, network_lifecycle.rs
- `image/` (re-exported from minibox-core)
- `testing/`: mocks/, fixtures/, helpers/, backend/, capability

**Features:** `test-utils` (mocks + fixtures + conformance), `metrics`
(Prometheus endpoint), `otel` (OTLP trace export).

**Benchmarks:** none — all criterion benches live in `crates/minibox-bench`.

---

## minibox-macros

Declarative macros for adapter boilerplate reduction.

**Macros:** `as_any!` (downcasting), `default_new!` (Default via new()),
`adapt!` (both), `provide!` (LLM provider constructors), `require_capability!`
(test gating), `normalize_name!`/`normalize_digest!`/`normalize!`/
`denormalize_digest!` (path normalization), `test_run!` (test DaemonRequest
builder).

---

## miniboxd

Daemon binary. Platform-dispatches: Unix (Linux + macOS) -> `run_daemon()`,
Windows -> `winbox::start()`.

**Key modules:** `adapter_registry.rs` (AdapterSuite enum, env-based
selection), `listener.rs` (UnixServerListener).

**Adapter suites:** native, gke, colima, smolvm (default), krun (fallback).

---

## macbox

macOS daemon entry point and Colima adapter wiring. smolvm and krun adapters
live in `smolbox` (see below).

**Backends:**

- **Colima**: `ColimaRegistry`, `ColimaRuntime`, `ColimaFilesystem`,
  `ColimaLimiter` -- delegates to `colima ssh`/limactl/nerdctl

---

## smolbox

smolvm and krun adapter implementations for macOS VM backends.

**Backends:**

- **smolvm**: `SmolVmRegistry`, `SmolVmRuntime`, `SmolVmFilesystem`,
  `SmolVmLimiter` -- lightweight Linux VMs with subsecond boot
- **krun**: `KrunRegistry`, `KrunRuntime`, `KrunFilesystem`, `KrunLimiter` --
  libkrun micro-VMs (HVF on macOS, KVM on Linux); structs live in
  `crates/smolbox/src/krun/`

---

## winbox

Phase 1 Windows stub. `start()` returns error unconditionally.

**Modules:** `hcs.rs` (stub), `wsl2.rs` (stub), `paths.rs` (Named Pipe
path), `preflight.rs` (detection stubs).

---

## mbx

CLI client. Connects to daemon via Unix socket, sends JSON requests, streams
responses.

**Subcommands:** run, ps, stop, pause, resume, rm, pull, exec, logs, events,
prune, rmi, sandbox, snapshot (save/restore/list), pipeline (run/list/show),
load, doctor, manifest, verify, diagnose, update, upgrade.

---

## minibox-crux-plugin

Crux plugin binary. Exposes minibox container operations (pull, run, ps, stop,
rm, pause, resume, image-ls, image-rm) over JSON-RPC stdio for integration with
the crux agentic DSL runtime.

**Depends on:** minibox-core, crux-plugin (git dep).

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
internally by `cargo xtask test-conformance`.

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

## ail

Placeholder binary for the agent-improvement loop. No implementation yet.

---

## xtask

Development tool. All CI gate commands.

**Key commands:** pre-commit, prepush, verify, lint, fix, coverage,
coverage-check, agentlint, build-test-image, setup-test-vm, test-in-vm,
test-linux, bump, promote, preflight, doctor, ci-watch, daily-orchestration,
council, bench, fuzz, borrow-fixtures, nuke-test-state, clean-artifacts,
cas-add, cas-check.

**Subcommand groups:** `test <suite>` (unit, conformance, krun-conformance,
turmoil, shuttle, property, quickcheck, integration, e2e, system-suite,
sandbox, gke-profile, gke-adapter), `check <target>` (stale-names,
protocol-drift, protocol-sites, adapter-coverage, no-unwrap, repo-clean),
`docs <action>` (audit, lint, update-date), `info <target>` (metrics,
context, changes).
