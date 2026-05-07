# Minibox Architecture Reference

> Generated 2026-04-27 from automated codebase analysis.
> Updated 2026-05-05: crate count, version, dep graph, default adapter, protocol counts.
> Updated 2026-05-06: GKE adapter ImagePusher wired (OciPushAdapter via oci_push_adapter).
> Updated 2026-05-07: colima push/commit/build corrected to -- (not wired in daemon); colima
> wiring note corrected (minibox crate, not macbox); vz wiring clarified (not selectable).

## Workspace Overview

10 crates, Rust 2024 edition, workspace version 0.24.0.

```
minibox-macros          (proc-macro, ~300 LOC)
    ^
minibox-core            (lib, ~12.6k LOC) — cross-platform types, domain traits, protocol, image ops
    ^
minibox                 (lib, ~21.5k LOC) — Linux adapters, daemon handler/server/state, testing infra
    ^     ^
macbox       winbox     (platform libs)   — macOS backends (colima/krun/smolvm/vz) | Windows stub
    ^          ^
miniboxd                (bin+lib, ~1.6k LOC) — daemon entry point, adapter DI composition root

mbx                     (bin, ~3.2k LOC) — CLI client, connects via Unix socket
minibox-crux-plugin     (bin) — crux plugin host; exposes minibox ops over JSON-RPC stdio
minibox-conformance     (bin, internal) — conformance test harness for adapter trait contracts
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
| minibox-conformance | minibox, minibox-core                                         |
| xtask               | (standalone)                                                  |

---

## Domain Traits (Hexagonal Ports)

All defined in `minibox-core/src/domain.rs` and re-exported via `minibox`.

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

| Trait          | Status                      |
| -------------- | --------------------------- |
| `TtyProvider`  | Defined, not wired          |
| `ExecProvider` | Superseded by `ExecRuntime` |

---

## Adapter Suite Coverage Matrix

| Trait              | native | gke  | colima | smolvm |  krun  |  vf  | hcs  | wsl2 | docker |
| ------------------ | :----: | :--: | :----: | :----: | :----: | :--: | :--: | :--: | :----: |
| ImageRegistry      |   Y    |  Y   |   Y    |   Y    |   Y    | stub | stub |  --  |   --   |
| RegistryRouter     |   Y    |  Y   |   Y    |   Y    |   Y    |  --  |  --  |  --  |   --   |
| ImageLoader        |   Y    |  Y   |   Y    |  noop  |  noop  |  --  |  --  |  --  |   --   |
| FilesystemProvider |   Y    |  Y   |   Y    |   Y    |   Y    | stub | stub | stub |  stub  |
| ResourceLimiter    |   Y    | noop |   Y    |  noop  |   Y    | stub | stub | stub |  stub  |
| ContainerRuntime   |   Y    |  Y   |   Y    |   Y    |   Y    | stub | stub | stub |  stub  |
| NetworkProvider    |   Y    | noop |  noop  |  noop  |  noop  |  --  |  --  |  --  |   --   |
| MetricsRecorder    |   Y    |  Y   |   --   |   Y    | noop\* |  --  |  --  |  --  |   --   |
| ExecRuntime        |   Y    |  --  |   --   |   --   |   --   |  --  |  --  |  --  |   --   |
| ImagePusher        |   Y    |  Y   |   --   |   --   |   --   |  --  |  --  |  --  |   --   |
| ContainerCommitter |   Y    |  --  |   --   |   --   |   --   |  --  |  --  |  --  |   --   |
| ImageBuilder       |   Y    |  --  |   --   |   --   |   --   |  --  |  --  |  --  |   --   |
| VmCheckpoint       |  noop  | noop |  noop  |  noop  |  noop  |  --  |  --  |  --  |   --   |

Key: **Y** = real impl wired, **noop** = no-op wired, **stub** = returns Err (library only),
**--** = not implemented

\*krun constructs its own `NoOpMetricsRecorder` internally rather than accepting the shared
broker — an inconsistency vs native/gke/smolvm.

### Wiring Status

| Suite                         | Wired in miniboxd                                | `MINIBOX_ADAPTER` value | Platform     |
| ----------------------------- | ------------------------------------------------ | ----------------------- | ------------ |
| native                        | `build_native_handler_dependencies`              | `native`                | Linux only   |
| gke                           | `build_gke_handler_dependencies`                 | `gke`                   | Linux only   |
| colima                        | `build_colima_handler_dependencies`              | `colima`                | Unix         |
| smolvm                        | `build_smolvm_handler_dependencies`              | `smolvm` (default)      | Unix         |
| krun                          | `build_krun_handler_dependencies`                | `krun` (fallback)       | Unix         |
| vz                            | **not wired** (no AdapterSuite variant)          | --                      | library only |
| vf, hcs, wsl2, docker_desktop | **not wired**                                    | --                      | library only |

---

## HandlerDependencies Structure

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

24 request variants, 22 response variants. Canonical source:
`minibox-core/src/protocol.rs`.

### DaemonRequest Variants

Run, Stop, PauseContainer, ResumeContainer, Remove, List, Pull, LoadImage,
Exec, SendInput, ResizePty, Push, Commit, Build, SubscribeEvents, Prune,
ListImages, RemoveImage, ContainerLogs, RunPipeline, SaveSnapshot,
RestoreSnapshot, ListSnapshots, Update

### DaemonResponse Variants

**Terminal** (end a request): ContainerCreated, Success, ContainerPaused,
ContainerResumed, ContainerList, ImageLoaded, ImageList, Error,
ContainerStopped, BuildComplete, Pruned, PipelineComplete, SnapshotSaved,
SnapshotRestored, SnapshotList

**Non-terminal** (streaming): ContainerOutput, ExecStarted, PushProgress,
BuildOutput, Event, LogLine, UpdateProgress

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
2. Daemon checks image cache, pulls from Docker Hub if missing
3. Creates overlay mount (lowerdir=layers, upperdir=container_rw)
4. `spawn_blocking` -> fork child with `clone(CLONE_NEWPID|NS|UTS|IPC|NET)`
5. Child: create cgroup, write PID, set limits, mount proc/sys/tmpfs,
   `pivot_root`, close extra FDs, `execve` user command
6. Parent: track PID, spawn reaper task
7. On exit: reaper updates state to Stopped

## State Persistence

`DaemonState` persists container records to disk (atomic rename) on every
add/remove. Records survive daemon restart; running processes do not reattach.
State machine: Created -> Running -> Paused -> Stopped (+ Failed, Orphaned).
