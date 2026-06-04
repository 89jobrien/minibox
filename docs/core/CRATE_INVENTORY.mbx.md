# Crate Inventory

## Summary

| Crate               | Type       | LOC    | Source files | Test files              | Features                  |
| ------------------- | ---------- | ------ | ------------ | ----------------------- | ------------------------- |
| minibox-core        | lib        | ~12.6k | 28           | 7 integration + inline  | test-utils, fuzzing       |
| minibox             | lib        | ~21.5k | 71           | 36 integration + inline | test-utils, metrics, otel |
| minibox-macros      | proc-macro | ~175   | 9            | 1 integration           | --                        |
| miniboxd            | bin+lib    | ~1.6k  | 4            | 7 integration + inline  | metrics, otel, tailnet    |
| macbox              | lib        | ~3.6k  | 16           | 4                       | --                        |
| smolbox             | lib        | ~148   | 4            | 2 integration           | --                        |
| winbox              | lib        | ~280   | 5            | 0                       | --                        |
| mbx                 | bin        | ~3.2k  | 18           | 2 integration + inline  | subprocess-tests          |
| minibox-crux-plugin | bin        | ~1.2k  | 2            | 1 integration           | --                        |
| minibox-testsuite   | lib+bin    | ~3.7k  | 27           | 3 integration           | --                        |
| xtask               | bin        | ~5k    | 15           | 0                       | --                        |

**Estimated total:** ~79k lines of Rust across 345 source files. All crates at
version 0.30.0 (xtask 0.1.0).

<!-- fact:workspace_version=0.30.0 -->

---

## minibox-core

Cross-platform shared types. Single source of truth for protocol, domain
traits, error types, image management, and the Unix socket client.

**Key modules:** `domain.rs` (all trait ports), `protocol.rs` (DaemonRequest /
DaemonResponse), `image/` (ImageStore, ImageRef, RegistryClient, layer
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

**Benchmarks:** `trait_overhead`, `protocol_codec` (criterion).

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

Daemon binary. Platform-dispatches: macOS -> `macbox::start()`, Windows ->
`winbox::start()`, Linux -> inline `run_daemon()`.

**Key modules:** `adapter_registry.rs` (AdapterSuite enum, env-based
selection), `listener.rs` (UnixServerListener).

**Adapter suites:** native, gke, colima, smolvm (default), krun (fallback).

---

## macbox

macOS daemon implementation.

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
prune, rmi, sandbox, snapshot (save/restore/list), load, diagnose, update, upgrade.

---

## minibox-crux-plugin

Crux plugin binary. Exposes minibox container operations (pull, run, ps, stop,
rm, pause, resume, image-ls, image-rm) over JSON-RPC stdio for integration with
the crux agentic DSL runtime.

**Depends on:** minibox-core, crux-plugin (git dep).

---

## minibox-testsuite

Conformance test harness for adapter trait contracts. Not published; used
internally by `cargo xtask test-conformance`.

**Binaries:** `run-conformance`, `generate-report`.

**Depends on:** minibox, minibox-core.

---

## xtask

Development tool. All CI gate commands.

**Key commands:** pre-commit, prepush, test-unit, test-conformance,
test-krun-conformance, test-property, test-integration, test-e2e, test-e2e-suite,
test-system-suite, test-sandbox, bench, bump, nuke-test-state, clean-artifacts,
lint-docs, preflight, doctor, check-stale-names.
