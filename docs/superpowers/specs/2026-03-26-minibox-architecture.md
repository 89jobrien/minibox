# Minibox Architecture Reference

**Date**: 2026-03-26
**Scope**: Comprehensive architecture documentation with Mermaid diagrams for the minibox container runtime
**Companion to**: `2026-03-26-minibox-diagrams.md` (ASCII version)

---

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Crate Dependency Graph](#2-crate-dependency-graph)
3. [Hexagonal Architecture](#3-hexagonal-architecture)
4. [Adapter Suite Selection](#4-adapter-suite-selection)
5. [Container Lifecycle](#5-container-lifecycle)
6. [Image Pull Pipeline](#6-image-pull-pipeline)
7. [Protocol Design](#7-protocol-design)
8. [Container State Machine](#8-container-state-machine)
9. [Overlay Filesystem](#9-overlay-filesystem)
10. [Linux Namespace Isolation](#10-linux-namespace-isolation)
11. [cgroups v2 Resource Control](#11-cgroups-v2-resource-control)
12. [Security Model](#12-security-model)
13. [Async/Sync Boundary](#13-asyncsync-boundary)
14. [Error Propagation](#14-error-propagation)
15. [Runtime Directory Layout](#15-runtime-directory-layout)
16. [Observability & Tracing](#16-observability--tracing)
17. [Platform Dispatch](#17-platform-dispatch)
18. [Testing Strategy](#18-testing-strategy)

---

## 1. System Overview

Minibox is a Docker-like container runtime written in Rust. It uses a daemon/client architecture where `miniboxd` listens on a Unix socket and `minibox` (the CLI) sends JSON commands. Containers are isolated using Linux namespaces, resource-limited via cgroups v2, and run on overlay filesystems built from OCI image layers pulled from Docker Hub or GHCR.

The workspace contains 11 crates organized in three tiers: binary entry points at the top, platform-specific library crates in the middle, and a shared cross-platform core at the bottom. This layering ensures that `miniboxd` compiles on macOS (dispatching to `macbox::start()`) even though full container functionality requires Linux.

```mermaid
graph TB
    subgraph "Binary Crates"
        miniboxd["miniboxd<br/><i>async daemon</i>"]
        cli["minibox-cli<br/><i>CLI client</i>"]
        bench["minibox-bench<br/><i>benchmarks</i>"]
    end

    subgraph "Platform Libraries"
        daemonbox["daemonbox<br/><i>handler, server, state</i>"]
        minibox["minibox<br/><i>Linux adapters + container</i>"]
        macbox["macbox<br/><i>macOS Colima</i>"]
        winbox["winbox<br/><i>Windows stub</i>"]
    end

    subgraph "Shared Core"
        core["minibox-core<br/><i>domain, protocol, image, preflight</i>"]
        macros["minibox-macros<br/><i>as_any! adapt!</i>"]
        client["minibox-client<br/><i>socket connection</i>"]
    end

    subgraph "Standalone Libraries"
        llm["minibox-llm<br/><i>multi-provider LLM client</i>"]
        secrets["minibox-secrets<br/><i>credential store</i>"]
    end

    miniboxd --> daemonbox
    miniboxd --> minibox
    miniboxd --> macbox
    miniboxd --> winbox
    cli --> client
    cli --> core
    client --> core
    daemonbox --> core
    daemonbox --> minibox
    minibox --> core
    minibox --> macros
    bench --> minibox
    bench --> core
```

---

## 2. Crate Dependency Graph

Each crate has a specific responsibility. The most important architectural decision is the split between `minibox-core` (cross-platform types) and `minibox` (Linux-specific implementations). minibox re-exports core's `domain`, `image`, and `protocol` modules — this is intentional because the `as_any!` and `adapt!` proc macros expand to `crate::domain::AsAny`, which resolves at the call site (minibox), not the defining crate (minibox-macros).

```mermaid
graph LR
    subgraph "Entry Points"
        D[miniboxd]
        C[minibox-cli]
    end

    subgraph "Infrastructure"
        DB[daemonbox]
        LB[minibox]
        MB[macbox]
        WB[winbox]
    end

    subgraph "Core"
        MC[minibox-core]
        MM[minibox-macros]
        CL[minibox-client]
    end

    subgraph "External"
        NIX[nix crate<br/><i>Linux syscalls</i>]
        REQW[reqwest<br/><i>HTTP client</i>]
        TOKIO[tokio<br/><i>async runtime</i>]
        SERDE[serde<br/><i>serialization</i>]
    end

    D --> DB
    D --> LB
    D --> MB
    D --> WB
    D --> TOKIO
    C --> CL
    C --> MC
    CL --> MC
    DB --> MC
    DB --> LB
    DB --> TOKIO
    LB --> MC
    LB --> MM
    LB --> NIX
    MC --> REQW
    MC --> SERDE

    style MC fill:#e1f5fe
    style LB fill:#fff3e0
    style DB fill:#f3e5f5
```

**Key re-export chain**: `minibox` re-exports `minibox_core::{domain, image, protocol}`. This is load-bearing — removing these re-exports breaks macro expansion in every adapter file. The `#[allow(clippy::crate_in_macro_def)]` suppression on `as_any!` is intentional; `crate` in `macro_rules!` resolves at the call site by design.

---

## 3. Hexagonal Architecture

Minibox follows hexagonal (ports and adapters) architecture. Domain traits define the boundary between business logic and infrastructure. The daemon's `main()` function acts as the composition root, wiring concrete adapters into the handler via dependency injection.

This means the handler never calls `mount()` or `clone()` directly — it calls `filesystem.setup_rootfs()` and `runtime.spawn_process()`, which are trait methods. Tests substitute mock adapters (behind the `test-utils` feature flag) that return canned responses without touching the kernel.

```mermaid
graph TB
    subgraph "Driving Adapters (inputs)"
        CLI["minibox-cli<br/><i>Unix socket client</i>"]
        SERVER["daemonbox/server.rs<br/><i>Unix socket listener</i>"]
        FUTURE_HTTP["Future: HTTP/gRPC API"]
    end

    subgraph "Domain Core (ports)"
        IR["ImageRegistry<br/><i>has_image, pull_image,<br/>get_image_layers</i>"]
        FP["FilesystemProvider<br/><i>setup_rootfs, pivot_root</i>"]
        RL["ResourceLimiter<br/><i>create, cleanup</i>"]
        CR["ContainerRuntime<br/><i>spawn_process</i>"]
    end

    subgraph "Driven Adapters (outputs)"
        DHR["DockerHubRegistry"]
        GHCR["GhcrRegistry"]
        OFS["OverlayFilesystem"]
        CG["CgroupV2Limiter"]
        LNR["LinuxNamespaceRuntime"]
        COL["ColimaRuntime"]
        PROOT["ProotRuntime"]
        COPYFS["CopyFilesystem"]
        NOOP["NoopLimiter"]
    end

    CLI --> SERVER
    SERVER --> IR
    SERVER --> FP
    SERVER --> RL
    SERVER --> CR

    IR -.-> DHR
    IR -.-> GHCR
    IR -.-> COL
    FP -.-> OFS
    FP -.-> COPYFS
    FP -.-> COL
    RL -.-> CG
    RL -.-> NOOP
    RL -.-> COL
    CR -.-> LNR
    CR -.-> PROOT
    CR -.-> COL

    style IR fill:#bbdefb
    style FP fill:#bbdefb
    style RL fill:#bbdefb
    style CR fill:#bbdefb
```

**Domain traits** (defined in `minibox-core/src/domain.rs`):

| Trait                | Methods                                       | Purpose                                |
| -------------------- | --------------------------------------------- | -------------------------------------- |
| `ImageRegistry`      | `has_image`, `pull_image`, `get_image_layers` | Abstract image storage and retrieval   |
| `FilesystemProvider` | `setup_rootfs`, `pivot_root`                  | Abstract container filesystem creation |
| `ResourceLimiter`    | `create`, `cleanup`                           | Abstract resource limit enforcement    |
| `ContainerRuntime`   | `spawn_process`                               | Abstract container process creation    |

---

## 4. Adapter Suite Selection

The `MINIBOX_ADAPTER` environment variable selects which set of adapters to wire at daemon startup. This is a full-suite swap — every trait gets a compatible implementation. The native suite requires root and a Linux kernel with namespace/cgroup/overlay support. The GKE suite runs unprivileged using proot (ptrace-based fake root) and filesystem copies instead of overlay mounts. The Colima suite delegates to Lima/nerdctl on macOS.

```mermaid
graph TD
    ENV["MINIBOX_ADAPTER env var"]

    ENV -->|"native (default)"| NATIVE
    ENV -->|"gke"| GKE
    ENV -->|"colima"| COLIMA

    subgraph NATIVE["Native Suite (Linux + root)"]
        N_REG["DockerHubRegistry + GhcrRegistry"]
        N_FS["OverlayFilesystem<br/><i>overlay mount</i>"]
        N_LIM["CgroupV2Limiter<br/><i>memory, cpu, pids</i>"]
        N_RT["LinuxNamespaceRuntime<br/><i>clone(2) syscall</i>"]
        N_AUTH["SO_PEERCRED UID==0"]
    end

    subgraph GKE["GKE Suite (unprivileged)"]
        G_REG["DockerHubRegistry + GhcrRegistry"]
        G_FS["CopyFilesystem<br/><i>cp -r, no overlay</i>"]
        G_LIM["NoopLimiter<br/><i>no cgroup access</i>"]
        G_RT["ProotRuntime<br/><i>ptrace-based fake root</i>"]
        G_AUTH["UID check skipped"]
    end

    subgraph COLIMA["Colima Suite (macOS)"]
        C_REG["ColimaRegistry<br/><i>limactl + nerdctl</i>"]
        C_FS["ColimaFilesystem"]
        C_LIM["ColimaLimiter"]
        C_RT["ColimaRuntime<br/><i>nerdctl exec + chroot</i>"]
        C_AUTH["SO_PEERCRED N/A"]
    end

    style NATIVE fill:#c8e6c9
    style GKE fill:#fff9c4
    style COLIMA fill:#e1bee7
```

**Library-only adapters** (not yet wired into miniboxd): `docker_desktop`, `wsl2`, `vf` (Virtualization.framework), `hcs` (Windows HCS). These exist as code in `minibox/src/adapters/` but have no entry in the adapter suite selection logic.

---

## 5. Container Lifecycle

This is the most complex flow in the system. A single `minibox run` command triggers image resolution, network pulls, filesystem setup, process isolation, I/O streaming, and cleanup — spanning 5 crates and crossing the async/sync boundary twice.

The key insight is that everything up to and including the `clone()` syscall runs in `tokio::task::spawn_blocking`, because `clone()` cannot be called from an async context (it would block a Tokio worker thread, starving the socket accept loop). After the child is spawned, two more blocking tasks run concurrently: one drains the output pipe, the other waits for the child to exit.

```mermaid
sequenceDiagram
    participant User
    participant CLI as minibox-cli
    participant Socket as Unix Socket
    participant Server as daemonbox/server
    participant Handler as daemonbox/handler
    participant Registry as ImageRegistry
    participant FS as FilesystemProvider
    participant CG as ResourceLimiter
    participant RT as ContainerRuntime
    participant Child as Container Process

    User->>CLI: minibox run alpine -- /bin/echo hello
    CLI->>Socket: Connect to /run/minibox/miniboxd.sock
    CLI->>Socket: {"type":"Run","image":"alpine",...,"ephemeral":true}\n

    Socket->>Server: accept()
    Server->>Server: SO_PEERCRED → verify UID==0
    Server->>Handler: handle_run_streaming()

    Handler->>Handler: Generate container ID (UUID hex)
    Handler->>Handler: Parse ImageRef (alpine → library/alpine:latest)

    Handler->>Registry: has_image("library/alpine", "latest")?
    Registry-->>Handler: false (not cached)
    Handler->>Registry: pull_image(image_ref)
    Registry->>Registry: Auth → Manifest → Download layers → Extract
    Registry-->>Handler: ImageMetadata

    Handler->>FS: setup_rootfs(layers, container_dir)
    FS->>FS: mount("overlay", merged, lowerdir=..., upperdir=..., workdir=...)
    FS-->>Handler: merged path

    Handler->>CG: create(id, {memory: 256MB, cpu: 100})
    CG->>CG: mkdir cgroup, write memory.max, cpu.weight, pids.max
    CG-->>Handler: cgroup path

    Handler->>Handler: Register state: "Created"
    Handler->>CLI: {"type":"ContainerCreated","id":"abc123..."}\n

    Handler->>RT: spawn_process(config) [spawn_blocking]
    RT->>RT: Create pipe, forget OwnedFds
    RT->>RT: Allocate 8 MiB stack
    RT->>Child: clone(NEWPID|NEWNS|NEWUTS|NEWIPC|NEWNET|SIGCHLD)

    Note over Child: PID 1 in new namespace
    Child->>Child: dup2(write_fd → stdout/stderr)
    Child->>Child: Write PID to cgroup.procs
    Child->>Child: sethostname("minibox")
    Child->>Child: mount("", "/", MS_REC|MS_PRIVATE)
    Child->>Child: pivot_root(merged, /.put_old)
    Child->>Child: mount proc, sysfs, devtmpfs
    Child->>Child: umount2(/.put_old), rmdir
    Child->>Child: close_extra_fds()
    Child->>Child: execvp("/bin/echo", ["hello"])

    RT-->>Handler: SpawnResult { pid, output_reader }
    Handler->>Handler: Update state: "Running"

    par Output Drain
        Handler->>Handler: [spawn_blocking] Read pipe chunks
        Handler->>CLI: {"type":"ContainerOutput","stream":"stdout","data":"aGVsbG8K"}\n
    and Reaper
        Handler->>Handler: [spawn_blocking] waitpid(pid)
    end

    CLI->>User: hello

    Handler->>Handler: Child exited (exit_code=0)
    Handler->>Handler: Network cleanup
    Handler->>CLI: {"type":"ContainerStopped","exit_code":0}\n
    Handler->>Handler: Auto-remove ephemeral state

    CLI->>User: exit(0)
```

**Critical implementation details:**

- **Pipe FDs across clone()**: Both parent and child get copies of pipe file descriptors after `clone()`. The parent must `std::mem::forget` the `OwnedFd` values before cloning to prevent double-close. The child then `dup2`s the write end into stdout/stderr slots and closes both raw FDs. The parent closes the write end after clone returns, keeping only the read end for output streaming.

- **pivot_root requires MS_PRIVATE**: After `CLONE_NEWNS`, the child inherits shared mount propagation from the parent. `pivot_root` fails with EINVAL unless `mount("", "/", MS_REC|MS_PRIVATE)` is called first inside the child.

- **close_extra_fds uses close_range fast path**: Tries `close_range(3, ~0U, 0)` first (kernel 5.9+), falls back to `/proc/self/fd` iteration. The fallback must collect FD numbers into a Vec before closing, because closing during iteration would close `ReadDir`'s own FD.

---

## 6. Image Pull Pipeline

Image pulls follow the OCI distribution spec. Authentication is anonymous for public images (Docker Hub returns a short-lived bearer token). The manifest declares layer digests and sizes; each layer is a gzipped tar archive that gets security-validated during extraction.

The security validation in `layer.rs` is one of the most critical code paths in the system. It prevents Zip Slip attacks (path traversal via `../`), host path leakage (absolute symlinks surviving pivot_root), privilege escalation (setuid binaries in image layers), and device node injection.

```mermaid
flowchart TD
    START["registry.pull_image(image_ref)"] --> AUTH

    subgraph AUTH["Step 1: Authentication"]
        A1["GET auth.docker.io/token<br/>?service=registry.docker.io<br/>&scope=repository:library/alpine:pull"]
        A2["Response: {token: 'eyJ...'}"]
        A1 --> A2
    end

    AUTH --> MANIFEST

    subgraph MANIFEST["Step 2: Manifest Fetch"]
        M1["GET registry-1.docker.io/v2/library/alpine/manifests/latest<br/>Accept: application/vnd.oci.image.manifest.v1+json<br/>Authorization: Bearer {token}"]
        M2["SECURITY: manifest size ≤ 10 MB"]
        M3["Parse OciManifest: config digest + layer digests"]
        M1 --> M2 --> M3
    end

    MANIFEST --> LAYERS

    subgraph LAYERS["Step 3: Layer Download (per layer)"]
        L1["GET /v2/library/alpine/blobs/{digest}"]
        L2["SECURITY: layer size ≤ 10 GB"]
        L3["Verify SHA256(download) == manifest digest"]
        L1 --> L2 --> L3
    end

    LAYERS --> EXTRACT

    subgraph EXTRACT["Step 4: Extraction + Security Validation"]
        E1["gzip decompress → tar iterate"]
        E2{"For each entry"}
        E3["Skip '.' and './' root entries"]
        E4["Reject paths with '..' components"]
        E5["Reject absolute paths"]
        E6["Reject Block/Char device nodes"]
        E7["Strip setuid/setgid bits"]
        E8["Rewrite absolute symlinks → relative"]
        E9["Canonicalize parent, verify within base_dir"]
        E10["Extract to /var/lib/minibox/images/{ns}/{name}/{digest}/"]

        E1 --> E2
        E2 --> E3 --> E4 --> E5 --> E6 --> E7 --> E8 --> E9 --> E10
    end

    style AUTH fill:#e3f2fd
    style MANIFEST fill:#e8f5e9
    style LAYERS fill:#fff3e0
    style EXTRACT fill:#fce4ec
```

**Absolute symlink rewriting** deserves special attention. OCI layers contain symlinks like `/bin/sh → /bin/busybox`. After `pivot_root`, these absolute paths would reference the container's own root, which is correct at runtime. But during extraction to the host filesystem, `strip_prefix("/")` gives a path relative to the extraction root, not the symlink's parent directory. The `relative_path(entry_dir, abs_target)` function computes the correct relative target (e.g., `busybox` when both are in `bin/`), preventing broken symlinks like `/bin/bin/busybox`.

---

## 7. Protocol Design

Minibox uses a newline-delimited JSON protocol over a Unix domain socket. Each message is a single JSON object terminated by `\n`, using serde's `#[serde(tag = "type")]` for tagged enum dispatch. This is intentionally simple — no framing, no length prefix, no binary protocol. The maximum request size is 1 MB.

For ephemeral containers (`ephemeral: true`), the protocol becomes a streaming channel: the daemon sends `ContainerCreated` once, then streams `ContainerOutput` messages (with base64-encoded stdout/stderr chunks) in real time, and finally sends `ContainerStopped` with the exit code. The CLI exits with the container's exit code.

```mermaid
sequenceDiagram
    participant CLI as minibox-cli
    participant Daemon as miniboxd

    Note over CLI,Daemon: Run (ephemeral, streaming)
    CLI->>Daemon: {"type":"Run","image":"alpine","ephemeral":true,...}
    Daemon->>CLI: {"type":"ContainerCreated","id":"abc123"}
    Daemon->>CLI: {"type":"ContainerOutput","stream":"stdout","data":"aGVsbG8K"}
    Daemon->>CLI: {"type":"ContainerOutput","stream":"stderr","data":"..."}
    Daemon->>CLI: {"type":"ContainerStopped","exit_code":0}

    Note over CLI,Daemon: Pull
    CLI->>Daemon: {"type":"Pull","image":"library/nginx","tag":"latest"}
    Daemon->>CLI: {"type":"Success","message":"pulled library/nginx:latest"}

    Note over CLI,Daemon: List
    CLI->>Daemon: {"type":"List"}
    Daemon->>CLI: {"type":"ContainerList","containers":[...]}

    Note over CLI,Daemon: Stop
    CLI->>Daemon: {"type":"Stop","id":"abc123"}
    Daemon->>CLI: {"type":"Success","message":"stopped abc123"}

    Note over CLI,Daemon: Remove
    CLI->>Daemon: {"type":"Remove","id":"abc123"}
    Daemon->>CLI: {"type":"Success","message":"removed abc123"}

    Note over CLI,Daemon: Error case
    CLI->>Daemon: {"type":"Run","image":"nonexistent",...}
    Daemon->>CLI: {"type":"Error","message":"image not found"}
```

**Request types** (`DaemonRequest` enum):

| Variant  | Fields                                                                                | Description                                      |
| -------- | ------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `Run`    | `image`, `tag`, `command`, `memory_limit_bytes`, `cpu_weight`, `ephemeral`, `network` | Create and start a container                     |
| `Pull`   | `image`, `tag`                                                                        | Download an image without running                |
| `Stop`   | `id`                                                                                  | Send SIGTERM then SIGKILL to a running container |
| `Remove` | `id`                                                                                  | Clean up a stopped container's resources         |
| `List`   | (none)                                                                                | List all tracked containers                      |

**Response types** (`DaemonResponse` enum):

| Variant            | Fields                                    | Description                                     |
| ------------------ | ----------------------------------------- | ----------------------------------------------- |
| `ContainerCreated` | `id`                                      | Container created, process about to spawn       |
| `ContainerOutput`  | `stream` (Stdout/Stderr), `data` (base64) | Real-time output chunk from ephemeral container |
| `ContainerStopped` | `exit_code`                               | Container process exited                        |
| `ContainerList`    | `containers: Vec<ContainerInfo>`          | All tracked containers                          |
| `Success`          | `message`                                 | Generic success acknowledgment                  |
| `Error`            | `message`                                 | Operation failed                                |

---

## 8. Container State Machine

Container state is tracked in-memory by `DaemonState` (a `HashMap<String, ContainerRecord>` behind a Tokio `RwLock`). State is persisted to `state.json` after every mutation, but the daemon does not recover in-flight containers on restart — PID tracking is lost, and orphaned containers are not cleaned up.

Ephemeral containers (the default for `minibox run`) skip the Stop/Remove lifecycle entirely — they auto-remove after the child process exits.

```mermaid
stateDiagram-v2
    [*] --> Created: add_container()<br/>UUID assigned, overlay mounted,<br/>cgroup created
    Created --> Running: set_container_pid()<br/>child process spawned,<br/>PID recorded
    Running --> Stopped: waitpid() returns<br/>child exited,<br/>reaper updates state
    Stopped --> [*]: remove_container()<br/>unmount overlay, clean cgroup,<br/>delete runtime state

    Running --> [*]: ephemeral auto-remove<br/>(no Stop/Remove needed)

    note right of Created
        No PID yet — container
        infrastructure is ready
        but no process running
    end note

    note right of Running
        Container executing
        user command. PID tracked
        for stop/signal.
    end note

    note right of Stopped
        Child exited. Resources
        may still exist (overlay,
        cgroup). Needs explicit
        remove or auto-cleanup.
    end note
```

**ContainerRecord** fields:

| Field             | Type          | Description                 |
| ----------------- | ------------- | --------------------------- |
| `info.id`         | `String`      | 16-character UUID hex       |
| `info.image`      | `String`      | `image:tag`                 |
| `info.command`    | `String`      | Space-separated command     |
| `info.state`      | `String`      | Created / Running / Stopped |
| `info.created_at` | `String`      | ISO 8601 timestamp          |
| `info.pid`        | `Option<u32>` | Host PID (set when Running) |
| `rootfs_path`     | `PathBuf`     | Overlay merged dir          |
| `cgroup_path`     | `PathBuf`     | cgroup v2 dir               |

---

## 9. Overlay Filesystem

Each container gets a layered filesystem built from OCI image layers (read-only) with a writable upper directory. Linux overlay filesystem unions these into a single view. The container sees a normal root filesystem; writes go to the upper layer without modifying image layers (copy-on-write semantics).

Mount flags include `MS_NOSUID` and `MS_NODEV` to prevent privilege escalation via setuid binaries or device nodes in the writable layer. After pivot_root, `/sys` is mounted read-only to prevent the container from manipulating cgroup controls.

```mermaid
graph TB
    subgraph "Container View (after pivot_root)"
        ROOT["/ (merged)"]
        BIN["/bin"]
        ETC["/etc"]
        USR["/usr"]
        PROC["/proc<br/><i>MS_NOSUID|MS_NODEV|MS_NOEXEC</i>"]
        SYS["/sys<br/><i>MS_RDONLY (prevent cgroup escape)</i>"]
        DEV["/dev<br/><i>MS_NOSUID|MS_NODEV|MS_NOEXEC</i>"]
        ROOT --> BIN & ETC & USR & PROC & SYS & DEV
    end

    subgraph "Overlay Mount"
        UPPER["upper/ (read-write)<br/><i>containers/{id}/upper</i><br/>All container writes go here"]
        WORK["work/ (overlay internal)<br/><i>containers/{id}/work</i>"]
        LAYER2["Layer 2 (read-only)<br/><i>images/.../sha256_222</i>"]
        LAYER1["Layer 1 (read-only)<br/><i>images/.../sha256_111</i>"]
    end

    ROOT -.->|"merged mount"| UPPER
    UPPER --> WORK
    WORK --> LAYER2
    LAYER2 --> LAYER1

    style UPPER fill:#fff3e0
    style LAYER2 fill:#e8f5e9
    style LAYER1 fill:#e8f5e9
    style PROC fill:#fce4ec
    style SYS fill:#fce4ec
```

**Mount command equivalent:**

```
mount -t overlay overlay \
  -o lowerdir=layer2:layer1,upperdir=upper,workdir=work \
  -o nosuid,nodev \
  merged/
```

**Cleanup sequence on container remove:**

1. `umount2(merged, MNT_DETACH)` — detach overlay mount
2. `rm -rf /var/lib/minibox/containers/{id}/` — delete upper/work/merged dirs
3. `rm -rf /sys/fs/cgroup/minibox/{id}/` — delete cgroup directory

---

## 10. Linux Namespace Isolation

The `clone()` syscall creates the container process in five new namespaces simultaneously. Each namespace provides a different dimension of isolation. The child process is PID 1 in its namespace (the init process), which has special signal handling semantics.

The 8 MiB stack is heap-allocated (not the thread stack) because `clone()` requires an explicit stack pointer for the child. A C-calling-convention trampoline function unwraps a Rust closure from a raw pointer and calls it — this is the bridge between the C `clone()` API and Rust's closure-based abstraction.

```mermaid
graph TB
    subgraph "Host Kernel"
        DAEMON["miniboxd (PID 1000, UID 0)"]
    end

    DAEMON -->|"clone(2)"| CLONE

    subgraph CLONE["Clone Flags"]
        F1["CLONE_NEWPID<br/><i>Child is PID 1</i>"]
        F2["CLONE_NEWNS<br/><i>Private mount tree</i>"]
        F3["CLONE_NEWUTS<br/><i>Own hostname</i>"]
        F4["CLONE_NEWIPC<br/><i>Isolated SHM/semaphores</i>"]
        F5["CLONE_NEWNET<br/><i>Empty network stack</i>"]
        F6["SIGCHLD<br/><i>Notify parent on exit</i>"]
    end

    CLONE --> CHILD

    subgraph CHILD["Container Process (PID 1 in new namespace)"]
        direction TB
        P1["PID namespace: /proc shows only container processes"]
        P2["Mount namespace: private propagation, pivot_root swaps rootfs"]
        P3["UTS namespace: hostname = 'minibox'"]
        P4["IPC namespace: isolated shared memory, semaphores"]
        P5["Network namespace: loopback only (no veth/bridge)"]
    end

    style CLONE fill:#e3f2fd
    style CHILD fill:#e8f5e9
```

**What each namespace isolates:**

| Namespace | Flag           | Isolation                                                                     |
| --------- | -------------- | ----------------------------------------------------------------------------- |
| PID       | `CLONE_NEWPID` | Process ID space. Container's PID 1 only sees its own descendants in `/proc`. |
| Mount     | `CLONE_NEWNS`  | Mount table. After `pivot_root`, the host filesystem is invisible.            |
| UTS       | `CLONE_NEWUTS` | Hostname and NIS domain. Container gets hostname "minibox".                   |
| IPC       | `CLONE_NEWIPC` | System V IPC, POSIX message queues. No cross-container shared memory.         |
| Network   | `CLONE_NEWNET` | Network interfaces, routing tables, iptables. Only loopback available.        |

**Missing namespaces** (potential future work):

- **User namespace** (`CLONE_NEWUSER`): Would enable rootless containers by mapping UID 0 inside to an unprivileged UID outside.
- **Cgroup namespace** (`CLONE_NEWCGROUP`): Would virtualize `/sys/fs/cgroup` so the container sees itself as the cgroup root.

---

## 11. cgroups v2 Resource Control

Minibox uses cgroups v2 (unified hierarchy) for resource limits. Each container gets its own cgroup directory under the minibox slice. The daemon moves itself into a `supervisor/` leaf cgroup at startup to comply with the "no internal process" rule — a cgroup v2 constraint that forbids a cgroup from simultaneously containing processes and child cgroups.

```mermaid
graph TB
    subgraph "cgroup v2 Hierarchy"
        ROOT["/sys/fs/cgroup/"]
        SLICE["minibox.slice/"]
        SERVICE["miniboxd.service/"]
        SUPER["supervisor/<br/><i>daemon PID lives here</i>"]
        C1["container_abc123/<br/>memory.max = 268435456<br/>cpu.weight = 100<br/>pids.max = 1024"]
        C2["container_def456/<br/>memory.max = max<br/>cpu.weight = 50<br/>pids.max = 1024"]
    end

    ROOT --> SLICE --> SERVICE
    SERVICE --> SUPER
    SERVICE --> C1
    SERVICE --> C2

    style SUPER fill:#e3f2fd
    style C1 fill:#fff3e0
    style C2 fill:#fff3e0
```

**Resource controls per container:**

| File         | Default              | Description                                                  |
| ------------ | -------------------- | ------------------------------------------------------------ |
| `memory.max` | `max` (unlimited)    | Hard memory limit. Kernel OOM-kills on breach.               |
| `cpu.weight` | `100`                | Relative CPU share (1–10000). Only matters under contention. |
| `pids.max`   | `1024`               | Maximum process count. Prevents fork bombs.                  |
| `io.max`     | (not set by default) | Block I/O limits. Requires real block device `MAJOR:MINOR`.  |

**Gotchas:**

- `io.max` requires the `MAJOR:MINOR` of a real block device. In Colima VMs, the virtio disk is `253:0` (vda), not `8:0` (sda). Use `find_first_block_device()` which reads `/sys/block/*/dev`.
- PID 0 is silently accepted by kernel 6.8 when written to `cgroup.procs`, but it's never valid. Minibox validates explicitly before writing.
- The "no internal process" rule is enforced by cgroup v2: if the daemon's PID is in `miniboxd.service/cgroup.procs`, container cgroup creation under `miniboxd.service/` fails. The `supervisor/` leaf cgroup solves this.

---

## 12. Security Model

Minibox's security model operates at four boundaries: the network (image pulls), the filesystem (tar extraction), the socket (client authentication), and the container (kernel isolation). Each boundary has specific defenses.

```mermaid
flowchart TB
    subgraph NETWORK["Network Boundary"]
        N1["Manifest size ≤ 10 MB"]
        N2["Layer size ≤ 10 GB"]
        N3["Total image ≤ 5 GB"]
        N4["SHA256 digest verification"]
    end

    subgraph TAR["Tar Extraction Boundary"]
        T1["Reject '..' path components<br/><i>(Zip Slip prevention)</i>"]
        T2["Reject absolute paths"]
        T3["Reject Block/Char device nodes"]
        T4["Strip setuid/setgid bits<br/><i>(privilege escalation prevention)</i>"]
        T5["Rewrite absolute symlinks → relative<br/><i>(host path leak prevention)</i>"]
        T6["Canonicalize + prefix check"]
    end

    subgraph SOCKET["Socket Boundary"]
        S1["SO_PEERCRED: kernel provides UID/PID"]
        S2["Reject UID ≠ 0"]
        S3["Socket permissions: 0600"]
        S4["Audit log: client UID/PID"]
    end

    subgraph CONTAINER["Container Boundary"]
        C1["5 namespaces: PID, MNT, UTS, IPC, NET"]
        C2["pivot_root: host FS invisible"]
        C3["/sys mounted read-only<br/><i>(no cgroup escape)</i>"]
        C4["close_extra_fds: no inherited host FDs"]
        C5["MS_PRIVATE: no mount propagation"]
        C6["pids.max=1024: fork bomb prevention"]
        C7["memory.max: OOM kill on breach"]
    end

    NETWORK --> TAR --> SOCKET --> CONTAINER

    subgraph MISSING["Known Gaps"]
        M1["No user namespace remapping<br/><i>(runs as root inside container)</i>"]
        M2["No seccomp BPF filter"]
        M3["No network bridge/veth<br/><i>(isolated but no egress)</i>"]
        M4["No AppArmor/SELinux profiles"]
    end

    style NETWORK fill:#e3f2fd
    style TAR fill:#fce4ec
    style SOCKET fill:#e8f5e9
    style CONTAINER fill:#fff3e0
    style MISSING fill:#ffebee
```

**Threat model summary:**

| Threat                          | Mitigation                                    | Location                   |
| ------------------------------- | --------------------------------------------- | -------------------------- |
| Path traversal (Zip Slip)       | Reject `..`, canonicalize, prefix check       | `layer.rs`                 |
| Absolute symlink host leak      | Rewrite to relative path                      | `layer.rs:relative_path()` |
| Device node injection           | Reject Block/Char entry types                 | `layer.rs`                 |
| Privilege escalation via setuid | Strip setuid/setgid bits from extracted files | `layer.rs`                 |
| Unauthorized daemon access      | SO_PEERCRED UID==0 check                      | `server.rs`                |
| Cgroup escape                   | /sys mounted read-only                        | `filesystem.rs`            |
| Fork bomb                       | pids.max=1024                                 | `cgroups.rs`               |
| Memory exhaustion               | memory.max enforcement                        | `cgroups.rs`               |
| Host FD leak to container       | close_range(3, ~0U) or /proc/self/fd scan     | `process.rs`               |
| Mount propagation escape        | MS_REC\|MS_PRIVATE before pivot_root          | `filesystem.rs`            |

---

## 13. Async/Sync Boundary

The daemon uses Tokio for async I/O (socket accept, read, write, HTTP requests) but container operations involve blocking Linux syscalls that cannot run in an async context. The `tokio::task::spawn_blocking` function bridges this gap by offloading blocking work to a dedicated thread pool.

This is a non-negotiable architectural rule: **clone/fork/exec, mount/umount, waitpid, and file I/O from pipes must always run in spawn_blocking**. Violating this starves the Tokio worker threads, causing the socket accept loop to hang and all clients to time out.

```mermaid
graph TB
    subgraph ASYNC["Tokio Async Runtime"]
        direction TB
        A1["server.rs: accept() loop"]
        A2["server.rs: read_line(), write()"]
        A3["handler.rs: mpsc channel send"]
        A4["handler.rs: select! on channels"]
        A5["registry.rs: reqwest HTTP calls"]
        A6["state.rs: RwLock operations"]
    end

    subgraph BRIDGE["spawn_blocking boundary"]
        B1["tokio::task::spawn_blocking"]
    end

    subgraph SYNC["Blocking Thread Pool"]
        direction TB
        S1["process.rs: clone(2), execvp()"]
        S2["process.rs: waitpid()"]
        S3["filesystem.rs: mount(), pivot_root(), umount2()"]
        S4["cgroups.rs: write cgroup files"]
        S5["layer.rs: tar decompress, SHA256, file I/O"]
        S6["handler.rs: pipe read() → base64 → ContainerOutput"]
    end

    ASYNC --> B1
    B1 --> SYNC

    style ASYNC fill:#e3f2fd
    style BRIDGE fill:#fff9c4
    style SYNC fill:#fff3e0
```

**Why this matters in practice:** A single `clone()` call that blocks a Tokio worker for 50ms is enough to delay all pending socket accepts. With 4 worker threads and 4 concurrent container creations, the entire daemon becomes unresponsive. The `spawn_semaphore` (max 100 concurrent spawns) provides backpressure on top of this.

---

## 14. Error Propagation

Errors propagate differently depending on where they originate. Infrastructure failures (overlay mount, cgroup creation, image pull) are caught by the handler's `?` operator, trigger cleanup, and result in `DaemonResponse::Error` sent to the client. Child process failures (exec not found, pivot_root failure) can only communicate via the exit code (127 by convention) because the child is in a forked process with no direct error channel back to the parent.

```mermaid
flowchart TB
    subgraph CHILD["Child Process (after clone)"]
        CE1["pivot_root fails → _exit(127)"]
        CE2["execvp fails → _exit(127)"]
        CE3["Normal exit → exit(N)"]
    end

    subgraph PARENT["Parent (spawn_blocking)"]
        PE1["waitpid() → exit_code"]
        PE2["Read output pipe (may have error msg)"]
    end

    subgraph HANDLER["Handler (async)"]
        HE1["send ContainerStopped{exit_code}"]
        HE2["Infrastructure error<br/>→ cleanup overlay, cgroup, state<br/>→ send Error{message}"]
    end

    subgraph CLI_ERR["CLI"]
        CLE1["ContainerStopped → exit(exit_code)"]
        CLE2["Error → eprintln → exit(1)"]
    end

    CE1 --> PE1
    CE2 --> PE1
    CE3 --> PE1
    PE1 --> HE1
    PE2 --> HE1
    HE1 --> CLE1
    HE2 --> CLE2

    style CHILD fill:#fce4ec
    style HANDLER fill:#e3f2fd
```

**Error handling conventions:**

- Every fallible operation uses `.context("description")?` (anyhow)
- No `.unwrap()` in production code — panics would crash all running containers
- Cleanup failures are logged at `warn!` level but don't propagate (best-effort cleanup)
- The handler catches all errors before they reach the socket write, converting them to `DaemonResponse::Error`

---

## 15. Runtime Directory Layout

Minibox uses three directory trees: runtime state, persistent storage, and cgroup control files. All paths are configurable via environment variables for testing and non-standard deployments.

```mermaid
graph TB
    subgraph RUNTIME["/run/minibox/ (MINIBOX_RUN_DIR)"]
        SOCK["miniboxd.sock<br/><i>Unix socket, 0600</i>"]
        CONTAINERS_RUN["containers/"]
        PID_FILE["{id}/pid<br/><i>host PID file</i>"]
        CONTAINERS_RUN --> PID_FILE
    end

    subgraph STORAGE["/var/lib/minibox/ (MINIBOX_DATA_DIR)"]
        IMAGES["images/"]
        NS["library/"]
        IMG["alpine/latest/"]
        MANIFEST_JSON["manifest.json"]
        LAYER_DIR["sha256_abc.../"]
        LAYER_CONTENTS["bin/, etc/, usr/"]

        CONTAINERS_DATA["containers/"]
        MERGED["{id}/merged/<br/><i>overlay mount point</i>"]
        UPPER["{id}/upper/<br/><i>writable layer</i>"]
        WORK["{id}/work/<br/><i>overlay workdir</i>"]

        STATE["state.json<br/><i>persisted container records</i>"]

        IMAGES --> NS --> IMG
        IMG --> MANIFEST_JSON
        IMG --> LAYER_DIR --> LAYER_CONTENTS
        CONTAINERS_DATA --> MERGED & UPPER & WORK
    end

    subgraph CGROUP["/sys/fs/cgroup/ (MINIBOX_CGROUP_ROOT)"]
        CG_SLICE["minibox.slice/miniboxd.service/"]
        CG_SUPER["supervisor/<br/><i>daemon PID</i>"]
        CG_CONTAINER["{id}/<br/>cgroup.procs<br/>memory.max<br/>cpu.weight<br/>pids.max"]
        CG_SLICE --> CG_SUPER & CG_CONTAINER
    end

    style RUNTIME fill:#e3f2fd
    style STORAGE fill:#e8f5e9
    style CGROUP fill:#fff3e0
```

**Environment variable overrides:**

| Variable               | Default (root)           | Default (non-root)  | Description                 |
| ---------------------- | ------------------------ | ------------------- | --------------------------- |
| `MINIBOX_DATA_DIR`     | `/var/lib/minibox`       | `~/.minibox/cache/` | Image and container storage |
| `MINIBOX_RUN_DIR`      | `/run/minibox`           | `/run/minibox`      | Socket and runtime state    |
| `MINIBOX_SOCKET_PATH`  | `$RUN_DIR/miniboxd.sock` | —                   | Unix socket path            |
| `MINIBOX_CGROUP_ROOT`  | `/sys/fs/cgroup/minibox` | —                   | cgroup root for containers  |
| `MINIBOX_SOCKET_MODE`  | `0600`                   | —                   | Socket file permissions     |
| `MINIBOX_SOCKET_GROUP` | (none)                   | —                   | Socket group ownership      |

---

## 16. Observability & Tracing

Minibox uses the `tracing` crate for structured logging. Events follow a strict convention: severity is disciplined (no `warn!` for normal operations), messages use `"subsystem: verb noun"` format, and structured data goes in key-value fields (never interpolated into the message string). This makes logs machine-queryable while remaining human-readable.

```mermaid
graph LR
    subgraph "Severity Levels"
        ERROR["error!<br/><i>Unrecoverable failures</i><br/>container init crash<br/>fatal exec error"]
        WARN["warn!<br/><i>Security rejections</i><br/>degraded behavior<br/>cleanup failures"]
        INFO["info!<br/><i>Lifecycle milestones</i><br/>start/stop, pull phases<br/>overlay mount, pivot_root"]
        DEBUG["debug!<br/><i>Implementation detail</i><br/>syscall args, byte counts<br/>state transitions"]
    end

    ERROR ~~~ WARN ~~~ INFO ~~~ DEBUG

    style ERROR fill:#ffcdd2
    style WARN fill:#fff9c4
    style INFO fill:#c8e6c9
    style DEBUG fill:#e0e0e0
```

**Correct usage:**

```rust
tracing::info!(
    container_id = %id,
    pid = pid.as_raw(),
    rootfs = %config.rootfs.display(),
    "container: process started"     // subsystem: verb noun
);
```

**Wrong usage:**

```rust
// Values embedded in message — not queryable by log aggregators
tracing::info!("Container {} started with PID {}", id, pid);
```

**Canonical structured fields:**

| Field                         | Type          | Context                         |
| ----------------------------- | ------------- | ------------------------------- |
| `container_id`                | `&str`        | All container operations        |
| `pid` / `child_pid`           | `u32` / `i32` | Process lifecycle, clone result |
| `clone_flags`                 | `i32`         | namespace.rs                    |
| `entry`                       | `&Path`       | Tar security events             |
| `kind`                        | `&EntryType`  | Device node rejection           |
| `target` / `rewritten_target` | `&Path`       | Symlink rewrite events          |
| `new_root`                    | `&Path`       | pivot_root destination          |
| `fds_closed`                  | `usize`       | close_extra_fds count           |
| `command`                     | `&str`        | Container entrypoint            |
| `rootfs`                      | `&Path`       | Container rootfs path           |
| `mode_before` / `mode_after`  | `u32`         | Permission bit changes (octal)  |

---

## 17. Platform Dispatch

The `miniboxd` binary compiles on all platforms. On Linux, it runs the full daemon. On macOS, it delegates to `macbox::start()` which uses Colima (a Lima-based container VM). On Windows, it delegates to `winbox::start()` (currently a stub). This dispatch happens at compile time via `#[cfg(target_os)]` attributes.

```mermaid
flowchart TD
    MAIN["miniboxd/src/main.rs<br/>fn main()"]

    MAIN -->|"#[cfg(target_os = 'linux')]"| LINUX
    MAIN -->|"#[cfg(target_os = 'macos')]"| MACOS
    MAIN -->|"#[cfg(target_os = 'windows')]"| WINDOWS

    subgraph LINUX["Linux Path"]
        L1["Select adapter suite (MINIBOX_ADAPTER)"]
        L2["Create directories, load state"]
        L3["Dependency injection (HandlerDependencies)"]
        L4["Bind Unix socket, set permissions"]
        L5["run_server() accept loop"]
    end

    subgraph MACOS["macOS Path"]
        M1["macbox::start()"]
        M2["Colima adapter suite"]
        M3["limactl + nerdctl delegation"]
    end

    subgraph WINDOWS["Windows Path"]
        W1["winbox::start()"]
        W2["Stub (not implemented)"]
    end

    L1 --> L2 --> L3 --> L4 --> L5
    M1 --> M2 --> M3
    W1 --> W2

    style LINUX fill:#c8e6c9
    style MACOS fill:#e1bee7
    style WINDOWS fill:#e0e0e0
```

**Compile gate convention:**

- `#[cfg(target_os = "linux")]` — Linux-only: container module (namespaces, cgroups, overlay, process)
- `#[cfg(unix)]` — Unix-wide: preflight checks, signal handling, socket permissions
- `#[cfg(test)]` — Test fixtures, mock adapters

**Platform crate naming**: `{platform}box` — `minibox`, `macbox`, `winbox`. This convention should be followed for future platforms.

---

## 18. Testing Strategy

Minibox uses a four-tier testing pyramid. Unit tests run anywhere (including macOS CI). Integration and E2E tests require a Linux host with root access and kernel features (cgroups v2, overlay, namespaces). Property-based tests use proptest to fuzz protocol serialization, state machine transitions, and handler edge cases.

```mermaid
graph TB
    subgraph E2E["E2E Tests (14)<br/><i>Linux + root</i>"]
        E1["Full daemon + CLI"]
        E2["pull, run, ps, stop, rm"]
        E3["Streaming output, exit codes"]
        E4["Concurrent containers"]
    end

    subgraph INTEGRATION["Integration Tests (24)<br/><i>Linux + root</i>"]
        I1["Cgroup: memory, CPU, pids, io.max"]
        I2["Overlay: mount, layer stacking, cleanup"]
        I3["Container: namespace isolation, pivot_root"]
    end

    subgraph PROPERTY["Property Tests (33)<br/><i>Any platform</i>"]
        P1["Protocol roundtrip fuzzing"]
        P2["State machine invariants"]
        P3["ImageRef parsing edge cases"]
        P4["Handler boundary conditions"]
    end

    subgraph UNIT["Unit + Conformance (257)<br/><i>Any platform (4 skipped macOS)</i>"]
        U1["155 lib tests"]
        U2["11 CLI tests"]
        U3["22 handler tests"]
        U4["16 conformance tests"]
        U5["13 minibox-llm tests"]
        U6["36 minibox-secrets tests"]
    end

    E2E --> INTEGRATION --> PROPERTY --> UNIT

    style E2E fill:#ffcdd2
    style INTEGRATION fill:#fff9c4
    style PROPERTY fill:#c8e6c9
    style UNIT fill:#e3f2fd
```

**Test commands:**

| Command                                        | Platform     | What it runs                            |
| ---------------------------------------------- | ------------ | --------------------------------------- |
| `cargo xtask test-unit` / `just test-unit`     | Any          | Unit + conformance (257 tests)          |
| `cargo xtask test-property`                    | Any          | Proptest property tests (33 tests)      |
| `just test-integration`                        | Linux + root | Cgroup + overlay integration (24 tests) |
| `just test-e2e` / `cargo xtask test-e2e-suite` | Linux + root | Full daemon + CLI (14 tests)            |
| `cargo xtask pre-commit`                       | macOS-safe   | fmt-check + clippy + release build      |
| `cargo xtask prepush`                          | Any          | nextest + llvm-cov coverage             |

**macOS quality gates** (run in CI on `macos-latest`):

```
cargo fmt --all --check
cargo clippy -p minibox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -- -D warnings
cargo xtask test-unit
```

**Testing gotchas:**

- `std::env::set_var`/`remove_var` are `unsafe` in Rust 2024 — wrap in `unsafe {}` and serialize with a `static Mutex<()>` guard
- Proptest `FileFailurePersistence` warning in integration tests — suppress with `failure_persistence: None`
- `DaemonState` fixture requires `ImageStore::new(tmp.join("images"))` + a `data_dir` path
- `CgroupManager::create()` runs `create_dir_all` before bounds checks — proptest cgroup tests need a real cgroup2 mount and root
