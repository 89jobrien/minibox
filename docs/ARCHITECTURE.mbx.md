# Minibox Architecture Reference

> Generated 2026-04-27 from automated codebase analysis.
> Updated 2026-05-05: crate count, version, dep graph, default adapter, protocol counts.
> Updated 2026-05-06: GKE adapter ImagePusher wired (OciPushAdapter via oci_push_adapter).
> Updated 2026-05-07: colima wiring note corrected (minibox crate, not macbox);
> vz wiring row corrected (selectable via macbox env-var branch, feature-gated,
> macOS only — not in AdapterSuite enum). Colima push/commit/build are now wired
> via macbox::build_colima_handler_dependencies.
> Updated 2026-05-08: vz adapter removed (code dropped); QEMU vm_image/vm_run xtask commands
> removed.

## Workspace Overview

10 crates, Rust 2024 edition, workspace version 0.30.0.

```
minibox-macros          (proc-macro, ~300 LOC)
    ^
minibox-core            (lib, ~12.6k LOC) — cross-platform types, domain traits, protocol, image ops
    ^
minibox                 (lib, ~21.5k LOC) — Linux adapters, daemon handler/server/state, testing infra
    ^     ^
macbox       winbox     (platform libs)   — macOS backends (colima/krun/smolvm) | Windows stub
    ^          ^
miniboxd                (bin+lib, ~1.6k LOC) — daemon entry point, adapter DI composition root

mbx                     (bin, ~3.2k LOC) — CLI client, connects via Unix socket
minibox-crux-plugin     (bin) — crux plugin host; exposes minibox ops over JSON-RPC stdio
minibox-testsuite       (bin, internal) — conformance test harness for adapter trait contracts
xtask                   (dev tool, ~5k LOC) — CI gates, test runners, bench, VM image build
```

### Cross-Crate Dependency Graph

| Crate               | Depends on (workspace)                                        |
| ------------------- | ------------------------------------------------------------- |
| minibox-macros      | --                                                            |
| minibox-core        | minibox-macros                                                |
| minibox             | minibox-core, minibox-macros                                  |
| macbox              | minibox, minibox-core                                         |
| winbox              | minibox, minibox-core                                         |
| miniboxd            | minibox, minibox-core (unix), macbox (unix), winbox (windows) |
| mbx                 | minibox-core                                                  |
| minibox-crux-plugin | minibox-core                                                  |
| minibox-testsuite   | minibox, minibox-core                                         |
| xtask               | (standalone)                                                  |

---

## Domain Traits (Hexagonal Ports)

Most are defined in `crates/minibox-core/src/domain.rs`; `NetworkProvider` is in
`crates/minibox-core/src/domain/networking.rs`. All are re-exported via
`crates/minibox/src/domain.rs`.

### Primary Ports (wired in HandlerDependencies)

| Trait                | Methods                                               | Used by                                 |
| -------------------- | ----------------------------------------------------- | --------------------------------------- |
| `ImageRegistry`      | `has_image`, `pull_image`, `get_image_layers`         | All adapter suites                      |
| `RegistryRouter`     | `route` (hostname -> registry)                        | All suites via `HostnameRegistryRouter` |
| `ImageLoader`        | `load_image` (local tarball)                          | native, gke, colima                     |
| `FilesystemProvider` | supertrait: `RootfsSetup + ChildInit`                 | All suites                              |
| `ResourceLimiter`    | `create`, `add_process`, `cleanup`                    | All suites (noop on gke/smolvm)         |
| `ContainerRuntime`   | `capabilities`, `spawn_process`, `wait_for_exit`      | All suites                              |
| `NetworkProvider`    | `setup`, `attach`, `cleanup`, `stats`                 | native (bridge/host/noop), others noop  |
| `MetricsRecorder`    | `increment_counter`, `record_histogram`, `set_gauge`  | native, gke, smolvm                     |
| `ExecRuntime`        | `run_in_container`                                    | native only                             |
| `ImagePusher`        | `push_image`                                          | native, colima                          |
| `ContainerCommitter` | `commit`                                              | native, colima                          |
| `ImageBuilder`       | `build_image`                                         | native, colima                          |
| `VmCheckpoint`       | `save_snapshot`, `restore_snapshot`, `list_snapshots` | noop everywhere                         |
| `PtyAllocator`       | `allocate`                                            | internal exec path                      |

### Extension Ports (defined, not in HandlerDependencies)

Defined in `crates/minibox-core/src/domain/extensions.rs`.

| Trait          | Status                      |
| -------------- | --------------------------- |
| `TtyProvider`  | Defined, not wired          |
| `ExecProvider` | Superseded by `ExecRuntime` |

---

## Adapter Suite Coverage Matrix

| Trait              | native | gke  | colima | smolvm | krun | vf   | hcs  | wsl2 | docker |
| ------------------ | :----: | :--: | :----: | :----: | :--: | :--: | :--: | :--: | :----: |
| ImageRegistry      |   Y    |  Y   |   Y    |   Y    |  Y   | stub | stub |  --  |   --   |
| RegistryRouter     |   Y    |  Y   |   Y    |   Y    |  Y   |  --  |  --  |  --  |   --   |
| ImageLoader        |   Y    |  Y   |   Y    | noop   | noop |  --  |  --  |  --  |   --   |
| FilesystemProvider |   Y    |  Y   |   Y    |   Y    |  Y   | stub | stub | stub | stub   |
| ResourceLimiter    |   Y    | noop |   Y    |   Y    |  Y   | stub | stub | stub | stub   |
| ContainerRuntime   |   Y    |  Y   |   Y    |   Y    |  Y   | stub | stub | stub | stub   |
| NetworkProvider    |   Y    | noop | noop   | noop   | noop |  --  |  --  |  --  |   --   |
| MetricsRecorder    |   Y    |  Y   | noop   |   Y    |  Y   |  --  |  --  |  --  |   --   |
| ExecRuntime        |   Y    |  --  |   --   |   --   |  --  |  --  |  --  |  --  |   --   |
| ImagePusher        |   Y    |  Y   |   Y    |   --   |  --  |  --  |  --  |  --  |   --   |
| ContainerCommitter |   Y    |  --  |   Y    |   --   |  --  |  --  |  --  |  --  |   --   |
| ImageBuilder       |   Y    |  --  |   Y    |   Y    |  --  |  --  |  --  |  --  |   --   |
| VmCheckpoint       | noop   | noop | noop   | noop   | noop |  --  |  --  |  --  |   --   |

Note: `vz` (VZ.framework) adapter was removed in 2026-05-08. See git history for prior state.

Key: **Y** = real impl wired, **noop** = no-op wired, **stub** = returns Err (library only),
**--** = not implemented

\*krun constructs its own `NoOpMetricsRecorder` internally rather than accepting the shared
broker — an inconsistency vs native/gke/smolvm.

### Wiring Status

All `build_*_handler_dependencies` functions live in
`crates/miniboxd/src/main.rs`. Adapter selection logic is in
`crates/miniboxd/src/adapter_registry.rs:adapter_from_env`.

| Suite                         | Wired in miniboxd                                | `MINIBOX_ADAPTER` value | Platform     |
| ----------------------------- | ------------------------------------------------ | ----------------------- | ------------ |
| native                        | `build_native_handler_dependencies`              | `native`                | Linux only   |
| gke                           | `build_gke_handler_dependencies`                 | `gke`                   | Linux only   |
| colima                        | `build_colima_handler_dependencies`              | `colima`                | Unix         |
| smolvm                        | `build_smolvm_handler_dependencies`              | `smolvm` (default)      | Unix         |
| krun                          | `build_krun_handler_dependencies`                | `krun` (fallback)       | Unix         |
| vf, hcs, wsl2, docker_desktop | **not wired**                                    | --                      | library only |

---

## HandlerDependencies Structure

(see `crates/minibox/src/daemon/handler/mod.rs:HandlerDependencies`)

```
HandlerDependencies
+-- ImageDeps
|   +-- registry_router: DynRegistryRouter
|   +-- image_loader: DynImageLoader
|   +-- image_gc: Arc<dyn ImageGarbageCollector>
|   +-- image_store: Arc<ImageStore>
+-- LifecycleDeps
|   +-- filesystem: DynFilesystemProvider
|   +-- resource_limiter: DynResourceLimiter
|   +-- runtime: DynContainerRuntime
|   +-- network_provider: DynNetworkProvider
|   +-- containers_base: PathBuf
|   +-- run_containers_base: PathBuf
+-- ExecDeps
|   +-- exec_runtime: Option<DynExecRuntime>
|   +-- pty_sessions: SharedPtyRegistry
+-- BuildDeps
|   +-- image_pusher: Option<DynImagePusher>
|   +-- commit_adapter: Option<DynContainerCommitter>
|   +-- image_builder: Option<DynImageBuilder>
+-- EventDeps
|   +-- event_sink: Arc<dyn EventSink>
|   +-- event_source: Arc<dyn EventSource>
|   +-- metrics: DynMetricsRecorder
+-- policy: ContainerPolicy
+-- checkpoint: DynVmCheckpoint
```

---

## Protocol (JSON-over-newline on Unix socket)

27 request variants, 28 response variants. Canonical source:
`crates/minibox-core/src/protocol.rs`.

### DaemonRequest Variants

Run, Stop, PauseContainer, ResumeContainer, Remove, List, Pull, LoadImage,
Exec, SendInput, ResizePty, Push, Commit, Build, SubscribeEvents, Prune,
ListImages, RemoveImage, ContainerLogs, RunPipeline, SaveSnapshot,
RestoreSnapshot, ListSnapshots, Update, GetManifest, VerifyManifest

### DaemonResponse Variants

**Terminal** (end a request): ContainerCreated, Success, ContainerPaused,
ContainerResumed, ContainerList, ImageLoaded, ImageList, Error,
ContainerStopped, BuildComplete, Pruned, PipelineComplete, SnapshotSaved,
SnapshotRestored, SnapshotList, Manifest, VerifyResult

**Non-terminal** (streaming): ContainerOutput, ExecStarted, PushProgress,
BuildOutput, Event, LogLine, UpdateProgress

---

## Execution Manifest

(see `crates/minibox-core/src/domain/execution_manifest.rs`,
persisted in `crates/minibox/src/daemon/handler/run.rs:prepare_run`)

Every container run produces a persisted `execution-manifest.json` at
`{containers_base}/{id}/execution-manifest.json` **before** the process
is spawned. The manifest captures every measured input:

| Field | Source |
|---|---|
| `subject.image_ref` | Image reference as provided |
| `subject.image.layer_digests` | Resolved layer paths |
| `runtime.command` | Command and arguments |
| `runtime.env[].value_digest` | SHA-256 of each env value (never plaintext) |
| `runtime.mounts` | Bind mount host/container paths + read-only flag |
| `runtime.resource_limits` | Memory limit, CPU weight |
| `runtime.network_mode` | Network isolation mode |
| `runtime.privileged` | Privileged mode flag |

### Workload Digest

A deterministic `sha256` digest computed from a stable JSON projection
that excludes volatile fields (`created_at`, `manifest_path`,
`workload_digest` itself). Equal semantic inputs always produce equal
digests. Canonical implementation: `ExecutionManifest::seal()` in
`crates/minibox-core/src/domain/execution_manifest.rs`.

### Execution Policy

`ExecutionPolicy` evaluates a manifest against a rule set:
allowed/denied image patterns, network mode restrictions, privileged
gate, memory limit cap, mount path prefix allowlist. Loaded from JSON.
Canonical implementation:
`crates/minibox-core/src/domain/execution_policy.rs`.

### CLI

- `mbx manifest <id>` — print the execution manifest as JSON.
- `mbx verify <id> --policy <file>` — evaluate policy, exit 0 (allow)
  or 1 (deny).

### Future: Attestation

The manifest format is designed for future integration with Sigstore
cosign or in-toto attestation frameworks. The sealed workload digest
serves as the attestation subject.

---

## Mock System

Two locations with significant duplication:

| Location                        | Style                        | Unique mocks                                                                       |
| ------------------------------- | ---------------------------- | ---------------------------------------------------------------------------------- |
| `minibox/src/adapters/mocks.rs` | `adapt!` macro               | `FailableFilesystemMock` runtime toggles                                           |
| `minibox/src/testing/mocks/`    | manual impl, per-trait files | `MockImageBuilder`, `MockExecRuntime`, `MockImagePusher`, `MockContainerCommitter` |

Duplicated across both: MockRegistry, MockFilesystem, MockLimiter, MockRuntime,
MockNetwork. Minor API differences (Location A has `with_empty_layers` on
MockRegistry; Location B has public state structs).

---

## Container Lifecycle Flow

1. CLI sends `Run` request via Unix socket
   (see `crates/minibox-core/src/protocol.rs:DaemonRequest::Run`)
2. Daemon checks image cache, pulls from Docker Hub if missing
   (see `crates/minibox/src/daemon/handler/image.rs`)
3. Creates overlay mount (lowerdir=layers, upperdir=container_rw)
   (see `crates/minibox/src/adapters/filesystem.rs:OverlayFilesystem`)
4. `spawn_blocking` -> fork child with
   `clone(CLONE_NEWPID|NS|UTS|IPC|NET)`
   (see `crates/minibox/src/container/namespace.rs`)
5. Child: create cgroup, write PID, set limits, mount proc/sys/tmpfs,
   `pivot_root`, close extra FDs, `execve` user command
   (see `crates/minibox/src/container/process.rs:child_init`,
   `crates/minibox/src/container/filesystem.rs`)
6. Parent: track PID, spawn reaper task
7. On exit: reaper updates state to Stopped

## State Persistence

`DaemonState`
(see `crates/minibox/src/daemon/state.rs:DaemonState`) persists
container records to disk (atomic rename) on every add/remove.
Records survive daemon restart; running processes do not reattach.
State machine: Created -> Running -> Paused -> Stopped
(+ Failed, Orphaned).
See `docs/STATE_MODEL.mbx.md` for full detail.

---

## Reference Documents

| Document                                                    | Purpose                                              |
| ----------------------------------------------------------- | ---------------------------------------------------- |
| [`docs/FEATURE_MATRIX.mbx.md`](FEATURE_MATRIX.mbx.md)      | Per-adapter capability matrix (authoritative)        |
| [`docs/GOTCHAS.mbx.md`](GOTCHAS.mbx.md)                    | Non-obvious Rust/container/protocol pitfalls         |
| [`docs/TEST_INFRASTRUCTURE.mbx.md`](TEST_INFRASTRUCTURE.mbx.md) | Test categories, CI coverage, xtask commands    |
| [`docs/STATE_MODEL.mbx.md`](STATE_MODEL.mbx.md)            | Daemon persistence model and state machine           |
| [`docs/SECURITY_INVARIANTS.mbx.md`](SECURITY_INVARIANTS.mbx.md) | Security rules to preserve across changes       |
| [`docs/verifiable-execution.mbx.md`](verifiable-execution.mbx.md) | Execution manifest format, attestation path |
