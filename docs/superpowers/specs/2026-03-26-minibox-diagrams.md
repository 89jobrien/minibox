# Minibox Architecture Diagrams

**Date**: 2026-03-26
**Scope**: Data flow, component relationships, and conventions for the minibox container runtime

---

## 1. System Overview — Crate Topology

```
┌────────────────────────────────────────────────────────────────────────────┐
│                              minibox workspace                             │
│                                                                            │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                         Binary Crates                                │  │
│  │                                                                      │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌───────────────────────────┐   │  │
│  │  │  miniboxd    │  │ minibox-cli  │  │  minibox-bench            │   │  │
│  │  │  (daemon)    │  │ (client)     │  │  (benchmarks)             │   │  │
│  │  └──────┬───────┘  └──────┬───────┘  └───────────────────────────┘   │  │
│  └─────────┼─────────────────┼──────────────────────────────────────────┘  │
│            │                 │                                             │
│  ┌─────────▼─────────────────▼───────────────────────────────────────────┐ │
│  │                       Library Crates                                  │ │
│  │                                                                       │ │
│  │  ┌────────────┐  ┌────────────┐  ┌────────┐  ┌────────┐               │ │
│  │  │ daemonbox  │  │  minibox  │  │ macbox │  │ winbox │               │ │
│  │  │ handler    │  │  adapters  │  │ colima │  │  stub  │               │ │
│  │  │ server     │  │  container │  │ macOS  │  │  win   │               │ │
│  │  │ state      │  │  image     │  │        │  │        │               │ │
│  │  └─────┬──────┘  └─────┬──────┘  └────────┘  └────────┘               │ │
│  │        │               │                                              │ │
│  │  ┌─────▼───────────────▼──────────────────────────────────────────┐   │ │
│  │  │                   minibox-core                                 │   │ │
│  │  │  domain.rs   protocol.rs   image/   preflight/   error.rs      │   │ │
│  │  └────────────────────────────────────────────────────────────────┘   │ │
│  │                                                                       │ │
│  │  ┌────────────────┐  ┌──────────────┐  ┌──────────────────────┐       │ │
│  │  │ minibox-macros │  │ minibox-llm  │  │  minibox-secrets     │       │ │
│  │  │ as_any! adapt! │  │ LLM client   │  │  credential store    │       │ │
│  │  └────────────────┘  └──────────────┘  └──────────────────────┘       │ │
│  └───────────────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Crate Dependency Graph

```
  miniboxd
  ├──→ daemonbox
  │    ├──→ minibox-core
  │    └──→ minibox
  ├──→ minibox
  │    ├──→ minibox-core
  │    ├──→ minibox-macros (proc-macro)
  │    └──→ nix (syscalls)
  ├──→ macbox (cfg target_os = "macos")
  └──→ winbox (cfg target_os = "windows")

  minibox-cli
  ├──→ minibox-client (socket connection)
  └──→ minibox-core (protocol types)

  minibox-core (zero infrastructure deps)
  ├── domain.rs         Trait definitions (ports)
  ├── protocol.rs       DaemonRequest / DaemonResponse
  ├── image/            ImageStore, RegistryClient, manifest, layer
  ├── preflight/        Host capability probing
  ├── error.rs          Cross-platform error types
  └── adapters/mocks    Test doubles (behind test-utils feature)

  minibox re-exports minibox-core:
    pub use minibox_core::domain;   ← as_any!/adapt! macros expand to crate::domain::AsAny
    pub use minibox_core::image;
    pub use minibox_core::protocol;

  ⚠ Do NOT remove minibox re-exports — macro expansion depends on them
```

---

## 3. Hexagonal Architecture — Ports and Adapters

```
                           ┌──────────────────────────────────┐
                           │      Domain Ports (Traits)       │
                           │       minibox-core/domain.rs     │
                           └──────────────┬───────────────────┘
                                          │
              ┌───────────────────────────┼───────────────────────────┐
              │                           │                           │
         Driving                     Domain Core                   Driven
         Adapters                    (pure logic)                  Adapters
              │                          │                            │
    ┌─────────▼──────────┐               │               ┌────────────▼─────────────┐
    │ minibox-cli        │               │               │ minibox/adapters/       │
    │ (CLI commands)     │               │               │                          │
    ├────────────────────┤               │               │ ┌────────────────────┐   │
    │ daemonbox/server   │               │               │ │  ImageRegistry     │   │
    │ (Unix socket)      │               │               │ │  ├─ DockerHub      │   │
    └────────────────────┘               │               │ │  ├─ GHCR           │   │
                                         │               │ │  └─ Colima         │   │
    ┌────────────────────┐               │               │ ├────────────────────┤   │
    │ Future:            │               │               │ │  ContainerRuntime  │   │
    │ • HTTP/gRPC API    │               │               │ │  ├─ LinuxNamespace │   │
    │ • miniboxctl REST      │               │               │ │  ├─ Proot (GKE)    │   │
    │ • Web dashboard    │               │               │ │  └─ Colima         │   │
    └────────────────────┘               │               │ ├────────────────────┤   │
                                         │               │ │  FilesystemProvider│   │
    ┌──────────────────────────────────────┐             │ │  ├─ Overlay        │   │
    │         Domain Traits                │             │ │  ├─ Copy (GKE)     │   │
    │                                      │             │ │  └─ Colima         │   │
    │  ImageRegistry                       │             │ ├────────────────────┤   │
    │  ├─ has_image(name, tag) → bool      │             │ │  ResourceLimiter   │   │
    │  ├─ pull_image(ref) → ImageMetadata  │             │ │  ├─ CgroupV2       │   │
    │  └─ get_image_layers(name, tag)      │             │ │  ├─ Noop (GKE)     │   │
    │         → Vec<PathBuf>               │             │ │  └─ Colima         │   │
    │                                      │             │ └────────────────────┘   │
    │  ContainerRuntime                    │             │                          │
    │  └─ spawn_process(config)            │             │ External deps:           │
    │         → SpawnResult                │             │ ├─ Linux kernel (clone)  │
    │                                      │             │ ├─ Docker Hub API        │
    │  FilesystemProvider                  │             │ ├─ GHCR API              │
    │  ├─ setup_rootfs(layers, dir)        │             │ ├─ overlayfs             │
    │  │       → PathBuf                   │             │ ├─ cgroup2 sysfs         │
    │  └─ pivot_root(new_root) → ()        │             │ └─ proot (GKE only)      │
    │                                      │             └──────────────────────────┘
    │  ResourceLimiter                     │
    │  ├─ create(id, config) → String      │
    │  └─ cleanup(id) → ()                 │
    └──────────────────────────────────────┘
```

---

## 4. Adapter Suite Selection (MINIBOX_ADAPTER)

```
  miniboxd/src/main.rs — Dependency Injection (lines 309–368)

  MINIBOX_ADAPTER env var
          │
          ├─ "native" (default, Linux root)
          │   ┌─────────────────────────────────────────────────────┐
          │   │  Registry:   DockerHubRegistry + GhcrRegistry       │
          │   │  Filesystem: OverlayFilesystem (overlay mount)      │
          │   │  Limiter:    CgroupV2Limiter (memory, cpu, pids)    │
          │   │  Runtime:    LinuxNamespaceRuntime (clone syscall)  │
          │   │  Network:    NoopNetwork                            │
          │   │  Auth:       SO_PEERCRED UID==0 required            │
          │   └─────────────────────────────────────────────────────┘
          │
          ├─ "gke" (unprivileged, GKE/Cloud Run)
          │   ┌─────────────────────────────────────────────────────┐
          │   │  Registry:   DockerHubRegistry + GhcrRegistry       │
          │   │  Filesystem: CopyFilesystem (cp -r, no overlay)     │
          │   │  Limiter:    NoopLimiter (no cgroup access)         │
          │   │  Runtime:    ProotRuntime (ptrace-based fake root)  │
          │   │  Network:    NoopNetwork                            │
          │   │  Auth:       SO_PEERCRED UID check skipped          │
          │   └─────────────────────────────────────────────────────┘
          │
          └─ "colima" (macOS via Colima VM)
              ┌─────────────────────────────────────────────────────┐
              │  Registry:   ColimaRegistry (limactl + nerdctl)     │
              │  Filesystem: ColimaFilesystem                       │
              │  Limiter:    ColimaLimiter                          │
              │  Runtime:    ColimaRuntime (nerdctl exec + chroot)  │
              │  Network:    NoopNetwork                            │
              │  Auth:       SO_PEERCRED not available on macOS     │
              └─────────────────────────────────────────────────────┘

  Not yet wired (library-only):
  ├─ docker_desktop  (adapters/docker_desktop.rs)
  ├─ wsl2            (adapters/wsl2.rs)
  ├─ vf              (adapters/vf.rs — Virtualization.framework)
  └─ hcs             (adapters/hcs.rs — Windows HCS)
```

---

## 5. Container Lifecycle — End to End

```
  $ minibox run alpine -- /bin/echo "hello"
  │
  ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ minibox-cli                                                 │
  │                                                             │
  │ 1. Parse args (clap)                                        │
  │ 2. Build DaemonRequest::Run {                               │
  │      image: "alpine", tag: "latest",                        │
  │      command: ["/bin/echo", "hello"],                       │
  │      ephemeral: true                                        │
  │    }                                                        │
  │ 3. Connect to /run/minibox/miniboxd.sock                    │
  │ 4. Send JSON + newline                                      │
  └───────────────────────────┬─────────────────────────────────┘
                              │ Unix socket
                              ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ daemonbox/server.rs                                         │
  │                                                             │
  │ 5. Accept connection                                        │
  │ 6. getsockopt(SO_PEERCRED) → uid, pid                       │
  │ 7. SECURITY: reject if uid != 0                             │
  │ 8. tokio::spawn → handle_connection()                       │
  │ 9. Read line, deserialize DaemonRequest                     │
  │ 10. Create mpsc channel (tx for responses)                  │
  │ 11. Dispatch to handler                                     │
  └───────────────────────────┬─────────────────────────────────┘
                              │
                              ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ daemonbox/handler.rs — handle_run_streaming()               │
  │                                                             │
  │ 12. Generate container ID (16-char UUID hex)                │
  │ 13. Parse ImageRef ("alpine" → library/alpine:latest)       │
  │ 14. Select registry (DockerHub vs GHCR)                     │
  │                                                             │
  │ ┌─ IMAGE PULL (if not cached) ──────────────────────────┐   │
  │ │ 15. Auth: GET auth.docker.io/token (anonymous)        │   │
  │ │ 16. Manifest: GET /v2/library/alpine/manifests/latest │   │
  │ │ 17. For each layer:                                   │   │
  │ │     GET /v2/library/alpine/blobs/{digest}             │   │
  │ │     Verify SHA256, extract tar (security validated)   │   │
  │ │ 18. Store at /var/lib/minibox/images/library/alpine/  │   │
  │ └───────────────────────────────────────────────────────┘   │
  │                                                             │
  │ 19. Create container dirs:                                  │
  │     /var/lib/minibox/containers/{id}/merged                 │
  │     /var/lib/minibox/containers/{id}/upper                  │
  │     /var/lib/minibox/containers/{id}/work                   │
  │                                                             │
  │ 20. OVERLAY MOUNT (FilesystemProvider):                     │
  │     mount("overlay", merged,                                │
  │       "lowerdir=layer1:layer2,upperdir=..,workdir=..",      │
  │       MS_NOSUID | MS_NODEV)                                 │
  │                                                             │
  │ 21. CGROUP SETUP (ResourceLimiter):                         │
  │     mkdir /sys/fs/cgroup/minibox/{id}/                      │
  │     write memory.max, cpu.weight, pids.max=1024             │
  │                                                             │
  │ 22. Register container state: "Created"                     │
  │ 23. Acquire spawn_semaphore (max 100 concurrent)            │
  │ 24. Send ContainerCreated{id} → client                      │
  └───────────────────────────┬─────────────────────────────────┘
                              │
                              ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ spawn_blocking → process.rs + namespace.rs                  │
  │                                                             │
  │ PARENT PROCESS:                                             │
  │ 25. Create output pipe (O_CLOEXEC)                          │
  │ 26. Extract raw FDs, forget OwnedFds (prevent double-close) │
  │ 27. Allocate 8 MiB stack on heap                            │
  │ 28. Call clone(2):                                          │
  │     CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWUTS               │
  │     CLONE_NEWIPC | CLONE_NEWNET | SIGCHLD                   │
  │ 29. Close write_fd in parent                                │
  │ 30. Return SpawnResult { pid, output_reader }               │
  │                                                             │
  │ CHILD PROCESS (PID 1 in new PID namespace):                 │
  │ 31. dup2(write_fd → stdout, stderr)                         │
  │ 32. Close write_fd and read_fd                              │
  │ 33. Write PID to cgroup.procs (join cgroup)                 │
  │ 34. sethostname("minibox")                                  │
  │ 35. PIVOT ROOT:                                             │
  │     a. mount("", "/", MS_REC | MS_PRIVATE)                  │
  │        ↑ required: pivot_root fails EINVAL on shared mounts │
  │     b. bind mount merged dir                                │
  │     c. mount proc  (MS_NOSUID | MS_NODEV | MS_NOEXEC)       │
  │     d. mount sysfs (MS_RDONLY — prevent cgroup escape)      │
  │     e. mount devtmpfs                                       │
  │     f. pivot_root(new_root, /.put_old)                      │
  │     g. chdir("/")                                           │
  │     h. umount2("/.put_old", MNT_DETACH)                     │
  │     i. rmdir("/.put_old")                                   │
  │ 36. close_extra_fds():                                      │
  │     close_range(3, ~0U, 0) (kernel 5.9+)                    │
  │     fallback: /proc/self/fd iteration                       │
  │ 37. execvp("/bin/echo", ["hello"])                          │
  │     ↓ (process replaced, never returns)                     │
  └───────────────────────────┬─────────────────────────────────┘
                              │
                              ▼
  ┌──────────────────────────────────────────────────────────────┐
  │ Concurrent tasks (handler.rs)                                │
  │                                                              │
  │ OUTPUT DRAIN (spawn_blocking):                               │
  │ 38. Read 4096-byte chunks from output pipe                   │
  │ 39. Base64 encode each chunk                                 │
  │ 40. Send ContainerOutput{stdout, data} → client              │
  │     (client decodes + prints to terminal in real time)       │
  │                                                              │
  │ REAPER (spawn_blocking):                                     │
  │ 41. waitpid(child_pid, 0) — block until exit                 │
  │ 42. Extract exit code                                        │
  │                                                              │
  │ COMPLETION:                                                  │
  │ 43. Wait for drain to finish                                 │
  │ 44. Network cleanup                                          │
  │ 45. Send ContainerStopped{exit_code} → client                │
  │ 46. Auto-remove ephemeral container state                    │
  └───────────────────────────┬──────────────────────────────────┘
                              │
                              ▼
  ┌──────────────────────────────────────────────────────────────┐
  │ minibox-cli                                                  │
  │                                                              │
  │ 47. Receive ContainerOutput → decode base64 → print          │
  │ 48. Receive ContainerStopped → exit(exit_code)               │
  │                                                              │
  │ Terminal output:                                             │
  │   hello                                                      │
  │   $ echo $?                                                  │
  │   0                                                          │
  └──────────────────────────────────────────────────────────────┘
```

---

## 6. Image Pull Pipeline

```
  registry.pull_image(&image_ref)
          │
          ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │ Step 1: Authentication                                              │
  │                                                                     │
  │  GET https://auth.docker.io/token                                   │
  │      ?service=registry.docker.io                                    │
  │      &scope=repository:library/alpine:pull                          │
  │                                                                     │
  │  Response: { "token": "eyJ..." }                                    │
  │  (anonymous for public images)                                      │
  └───────────────────────────┬─────────────────────────────────────────┘
                              │
                              ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │ Step 2: Manifest Fetch                                              │
  │                                                                     │
  │  GET https://registry-1.docker.io/v2/library/alpine/manifests/latest│
  │  Accept: application/vnd.oci.image.manifest.v1+json                 │
  │  Authorization: Bearer {token}                                      │
  │                                                                     │
  │  SECURITY: manifest size ≤ 10 MB                                    │
  │                                                                     │
  │  Response: OciManifest {                                            │
  │    config: { digest: "sha256:abc..." },                             │
  │    layers: [                                                        │
  │      { digest: "sha256:111...", size: 3145728 },                    │
  │      { digest: "sha256:222...", size: 1048576 },                    │
  │    ]                                                                │
  │  }                                                                  │
  └───────────────────────────┬─────────────────────────────────────────┘
                              │
                              ▼
  ┌──────────────────────────────────────────────────────────────────────┐
  │ Step 3: Layer Download + Extraction (per layer, bottom to top)       │
  │                                                                      │
  │  GET /v2/library/alpine/blobs/sha256:111...                          │
  │  Authorization: Bearer {token}                                       │
  │                                                                      │
  │  SECURITY: layer size ≤ 10 GB                                        │
  │  Verify: SHA256(downloaded) == digest from manifest                  │
  │                                                                      │
  │  ┌───────────────────────────────────────────────────────────────┐   │
  │  │ extract_layer() — layer.rs                                    │   │
  │  │                                                               │   │
  │  │  gzip decompress → tar iterate entries:                       │   │
  │  │                                                               │   │
  │  │  for each entry:                                              │   │
  │  │    ┌────────────────────────────────────────────────────────┐ │   │
  │  │    │ SECURITY VALIDATION                                    │ │   │
  │  │    │                                                        │ │   │
  │  │    │  ✗ skip "." and "./" (tar root entries)                │ │   │
  │  │    │  ✗ reject paths with ".." components                   │ │   │
  │  │    │  ✗ reject absolute paths                               │ │   │
  │  │    │  ✗ reject Block and Char device nodes                  │ │   │
  │  │    │  ✗ reject named pipes                                  │ │   │
  │  │    │  ✓ strip setuid/setgid bits (04000, 02000, 01000)      │ │   │
  │  │    │  ✓ rewrite absolute symlinks to relative:              │ │   │
  │  │    │    /usr/bin/env → ../../usr/bin/env                    │ │   │
  │  │    │    (prevents host path leak after pivot_root)          │ │   │
  │  │    │  ✓ canonicalize parent, verify within base_dir         │ │   │
  │  │    └────────────────────────────────────────────────────────┘ │   │
  │  │                                                               │   │
  │  │  Extract to: /var/lib/minibox/images/{ns}/{name}/{digest}/    │   │
  │  └───────────────────────────────────────────────────────────────┘   │
  └──────────────────────────────────────────────────────────────────────┘
```

---

## 7. Protocol — JSON-over-Newline on Unix Socket

```
  minibox-cli                                          miniboxd
  ══════════                                          ════════
       │                                                  │
       │  ──── DaemonRequest::Run ────────────────────▶   │
       │  {"type":"Run","image":"alpine","tag":"latest",  │
       │   "command":["/bin/echo","hello"],               │
       │   "ephemeral":true}\n                            │
       │                                                  │
       │  ◀── DaemonResponse::ContainerCreated ────────   │
       │  {"type":"ContainerCreated",                     │
       │   "id":"abc1234567890def"}\n                     │
       │                                                  │
       │  ◀── DaemonResponse::ContainerOutput ─────────   │
       │  {"type":"ContainerOutput",                      │  (0+ messages,
       │   "stream":"stdout",                             │   streamed in
       │   "data":"aGVsbG8K"}\n                           │   real time)
       │                                                  │
       │  ◀── DaemonResponse::ContainerStopped ────────   │
       │  {"type":"ContainerStopped",                     │
       │   "exit_code":0}\n                               │
       │                                                  │

  Encoding:
  • Each message is a single JSON object terminated by \n
  • Tagged union via #[serde(tag = "type")]
  • MAX_REQUEST_SIZE = 1 MB
  • Binary data (stdout/stderr) is base64 encoded in ContainerOutput
  • Streaming: ephemeral=true triggers ContainerOutput stream

  Request Types:
  ┌──────────────────────────────────────────────────────────────┐
  │  Run    { image, tag, command, memory_limit_bytes,           │
  │           cpu_weight, ephemeral, network }                   │
  │  Pull   { image, tag }                                       │
  │  Stop   { id }                                               │
  │  Remove { id }                                               │
  │  List   { }                                                  │
  └──────────────────────────────────────────────────────────────┘

  Response Types:
  ┌──────────────────────────────────────────────────────────────┐
  │  ContainerCreated  { id }                                    │
  │  ContainerOutput   { stream: Stdout|Stderr, data: base64 }   │
  │  ContainerStopped  { exit_code }                             │
  │  ContainerList     { containers: Vec<ContainerInfo> }        │
  │  Success           { message }                               │
  │  Error             { message }                               │
  └──────────────────────────────────────────────────────────────┘
```

---

## 8. Container State Machine

```
                    ┌────────────────┐
                    │   (not exist)  │
                    └───────┬────────┘
                            │ add_container()
                            │ (UUID assigned, overlay mounted,
                            │  cgroup created, no PID yet)
                            ▼
                    ┌────────────────┐
                    │    Created     │
                    └───────┬────────┘
                            │ set_container_pid()
                            │ (child process spawned, PID recorded)
                            ▼
                    ┌────────────────┐
                    │    Running     │──── container executing user command
                    └───────┬────────┘
                            │ waitpid() returns
                            │ (child exited, reaper updates state)
                            ▼
                    ┌────────────────┐
                    │    Stopped     │──── still tracked, resources may remain
                    └───────┬────────┘
                            │ remove_container()
                            │ (cleanup overlay, cgroup, runtime state)
                            ▼
                    ┌────────────────┐
                    │   (removed)    │
                    └────────────────┘

  Ephemeral containers (ephemeral=true):
    Created → Running → Stopped → (auto-removed)
    No Stop/Remove commands needed — lifecycle is one-shot

  Persistence:
    DaemonState → state.json (written after every mutation)
    ⚠ Daemon restart loses in-memory PID tracking
    ⚠ Orphaned containers not cleaned up on restart
```

---

## 9. Overlay Filesystem Stack

```
  mount("overlay", merged, "overlay",
        MS_NOSUID | MS_NODEV,
        "lowerdir=layer2:layer1,upperdir=upper,workdir=work")


  ┌────────────────────────────────────────────────────────┐
  │ Container View (after pivot_root)                      │
  │                                                        │
  │   /                                                    │
  │   ├── bin/                                             │
  │   ├── etc/                                             │
  │   ├── usr/                                             │
  │   ├── proc/  (mounted, MS_NOSUID|MS_NODEV|MS_NOEXEC)   │
  │   ├── sys/   (mounted, MS_RDONLY — no cgroup escape)   │
  │   ├── dev/   (mounted, MS_NOSUID|MS_NODEV|MS_NOEXEC)   │
  │   └── tmp/                                             │
  └───────────────────────────┬────────────────────────────┘
                              │ merged =
                              │ /var/lib/minibox/containers/{id}/merged
                              │
  ┌───────────────────────────▼─────────────────────────────┐
  │                   Overlay Mount                         │
  │                                                         │
  │  ┌───────────────────────────────────────────────────┐  │
  │  │  upper (read-write)                               │  │
  │  │  /var/lib/minibox/containers/{id}/upper           │  │
  │  │  ← all container writes go here                   │  │
  │  └───────────────────────────────────────────────────┘  │
  │                         │                               │
  │  ┌──────────────────────▼────────────────────────────┐  │
  │  │  work (overlay internal bookkeeping)              │  │
  │  │  /var/lib/minibox/containers/{id}/work            │  │
  │  └───────────────────────────────────────────────────┘  │
  │                         │                               │
  │  ┌──────────────────────▼────────────────────────────┐  │
  │  │  lower (read-only image layers, bottom to top)    │  │
  │  │                                                   │  │
  │  │  layer 2: /var/lib/minibox/images/.../sha256_222  │  │
  │  │  layer 1: /var/lib/minibox/images/.../sha256_111  │  │
  │  │                                                   │  │
  │  │  lowerdir=layer2:layer1 (topmost first in mount)  │  │
  │  └───────────────────────────────────────────────────┘  │
  └─────────────────────────────────────────────────────────┘


  Cleanup on container remove:
    1. umount2(merged, MNT_DETACH)
    2. rm -rf /var/lib/minibox/containers/{id}/
    3. Remove cgroup dir
```

---

## 10. Namespace Isolation

```
  Host Kernel
  ═══════════

  miniboxd (PID 1000, UID 0)
  │
  ├── clone(2) with flags:
  │   ├── CLONE_NEWPID   → new PID namespace (child is PID 1)
  │   ├── CLONE_NEWNS    → new mount namespace (private mounts)
  │   ├── CLONE_NEWUTS   → new UTS namespace (own hostname)
  │   ├── CLONE_NEWIPC   → new IPC namespace (isolated semaphores/shm)
  │   ├── CLONE_NEWNET   → new network namespace (isolated stack)
  │   └── SIGCHLD        → parent gets SIGCHLD on child exit
  │
  └── Child Process (container)
      │
      ├── PID namespace:   PID 1 (init process of container)
      │   └── /proc shows only container processes
      │
      ├── Mount namespace: private mount propagation
      │   └── pivot_root swaps rootfs to overlay merged dir
      │   └── host mounts invisible after pivot
      │
      ├── UTS namespace:   hostname = "minibox"
      │
      ├── IPC namespace:   isolated shared memory, semaphores
      │
      └── Network namespace: empty (no veth/bridge configured)
          └── loopback only
          └── ⚠ No networking — isolated but not connected


  Stack allocation for clone:

  ┌─────────────────────────┐  ← stack_top (clone starts here)
  │                         │
  │     8 MiB stack         │  heap-allocated Vec<u8>
  │     (grows downward)    │
  │                         │
  └─────────────────────────┘  ← stack_bottom

  Clone trampoline:
    extern "C" fn trampoline(arg: *mut c_void) → c_int
    ├── cast arg back to Box<dyn FnMut() → isize>
    ├── call closure (child_fn)
    └── return exit code
```

---

## 11. cgroups v2 Resource Limits

```
  /sys/fs/cgroup/
  └── minibox.slice/
      └── miniboxd.service/
          ├── supervisor/                ← daemon's own leaf cgroup
          │   └── cgroup.procs = {miniboxd PID}
          │
          ├── {container_id_1}/          ← per-container cgroup
          │   ├── cgroup.procs = {child PID}
          │   ├── memory.max = 268435456     (256 MB)
          │   ├── cpu.weight = 100           (relative weight)
          │   ├── pids.max = 1024            (fork bomb prevention)
          │   └── io.max = "253:0 rbps=X"   (optional, block device)
          │
          └── {container_id_2}/
              ├── cgroup.procs = {child PID}
              ├── memory.max = max           (unlimited)
              ├── cpu.weight = 50
              └── pids.max = 1024

  Key constraint: cgroup v2 "no internal process" rule
  ├── A cgroup cannot have BOTH processes AND child cgroups
  ├── Daemon moves itself to supervisor/ leaf at startup
  └── Container cgroups are siblings, not children of daemon's cgroup

  CgroupManager operations:
  ┌──────────────────────────────────────────────────────────┐
  │ create(id, config):                                      │
  │   mkdir /sys/fs/cgroup/minibox/{id}/                     │
  │   enable controllers on parent (subtree_control)         │
  │   write memory.max (if limit set)                        │
  │   write cpu.weight (if limit set)                        │
  │   write pids.max = 1024 (always)                         │
  │                                                          │
  │ join(id, pid):                                           │
  │   write pid to cgroup.procs                              │
  │   SECURITY: validate pid != 0 (kernel 6.8 silently       │
  │   accepts 0 but it's never valid)                        │
  │                                                          │
  │ cleanup(id):                                             │
  │   rm -rf /sys/fs/cgroup/minibox/{id}/                    │
  │   (best-effort, warn on failure)                         │
  └──────────────────────────────────────────────────────────┘
```

---

## 12. Security Boundaries

```
  ┌─────────────────────────────────────────────────────────────────┐
  │                    SECURITY BOUNDARY MAP                        │
  │                                                                 │
  │  ┌─── NETWORK BOUNDARY ────────────────────────────────────┐    │
  │  │                                                         │    │
  │  │  Docker Hub / GHCR API                                  │    │
  │  │  ├─ Manifest size ≤ 10 MB (prevents memory exhaustion)  │    │
  │  │  ├─ Layer size ≤ 10 GB per layer                        │    │
  │  │  ├─ Total image size ≤ 5 GB                             │    │
  │  │  └─ SHA256 digest verification on every layer           │    │
  │  │                                                         │    │
  │  └─────────────────────────────────────────────────────────┘    │
  │                              │                                  │
  │  ┌─── TAR EXTRACTION ────────▼─────────────────────────────┐    │
  │  │                                                         │    │
  │  │  layer.rs — validate_tar_entry_path()                   │    │
  │  │  ├─ Reject ".." path components (Zip Slip)              │    │
  │  │  ├─ Reject absolute paths                               │    │
  │  │  ├─ Reject device nodes (Block, Char)                   │    │
  │  │  ├─ Reject named pipes                                  │    │
  │  │  ├─ Strip setuid/setgid bits                            │    │
  │  │  ├─ Rewrite absolute symlinks → relative                │    │
  │  │  └─ Canonicalize + prefix check on parent dir           │    │
  │  │                                                         │    │
  │  └─────────────────────────────────────────────────────────┘    │
  │                              │                                  │
  │  ┌─── SOCKET AUTH ───────────▼─────────────────────────────┐    │
  │  │                                                         │    │
  │  │  server.rs — SO_PEERCRED                                │    │
  │  │  ├─ Kernel provides UID/PID of connecting process       │    │
  │  │  ├─ Only UID 0 (root) can connect                       │    │
  │  │  ├─ Socket permissions: 0600 (owner-only)               │    │
  │  │  └─ Client UID/PID logged for audit trail               │    │
  │  │                                                         │    │
  │  └─────────────────────────────────────────────────────────┘    │
  │                              │                                  │
  │  ┌─── CONTAINER ISOLATION ───▼─────────────────────────────┐    │
  │  │                                                         │    │
  │  │  Namespaces: PID, MNT, UTS, IPC, NET (5 of 7)           │    │
  │  │  ├─ pivot_root: host FS invisible after swap            │    │
  │  │  ├─ /sys mounted read-only (no cgroup writes)           │    │
  │  │  ├─ close_extra_fds: no inherited host FDs              │    │
  │  │  └─ MS_PRIVATE: no mount propagation to host            │    │
  │  │                                                         │    │
  │  │  cgroups v2:                                            │    │
  │  │  ├─ memory.max: OOM kill on breach                      │    │
  │  │  ├─ cpu.weight: fair scheduling                         │    │
  │  │  └─ pids.max=1024: fork bomb prevention                 │    │
  │  │                                                         │    │
  │  │  ⚠ Missing:                                             │    │
  │  │  ├─ No user namespace remapping (runs as root inside)   │    │
  │  │  ├─ No seccomp BPF filter                               │    │
  │  │  └─ No network bridge/veth (isolated but no egress)     │    │
  │  │                                                         │    │
  │  └─────────────────────────────────────────────────────────┘    │
  └─────────────────────────────────────────────────────────────────┘
```

---

## 13. Async/Sync Boundary

```
  ┌───────────────────────────────────────────────────────────────┐
  │                     TOKIO ASYNC RUNTIME                       │
  │                                                               │
  │  server.rs                   handler.rs                       │
  │  ┌──────────────────┐       ┌──────────────────────────┐      │
  │  │ accept() loop    │       │ handle_run_streaming()   │      │
  │  │ read_line()      │       │ mpsc::channel            │      │
  │  │ write()          │       │ tx.send()                │      │
  │  │ SO_PEERCRED      │       │ select!                  │      │
  │  └──────────────────┘       └───────────┬──────────────┘      │
  │                                         │                     │
  │                              ┌──────────▼──────────────┐      │
  │                              │ tokio::spawn_blocking   │      │
  │                              │ (crosses async/sync     │      │
  │                              │  boundary)              │      │
  │                              └──────────┬──────────────┘      │
  └─────────────────────────────────────────┼─────────────────────┘
                                            │
  ┌─────────────────────────────────────────▼─────────────────────┐
  │                  BLOCKING THREAD POOL                         │
  │                                                               │
  │  process.rs                filesystem.rs                      │
  │  ┌──────────────────┐     ┌──────────────────┐                │
  │  │ clone(2)         │     │ mount()          │                │
  │  │ waitpid()        │     │ pivot_root()     │                │
  │  │ execvp()         │     │ umount2()        │                │
  │  └──────────────────┘     └──────────────────┘                │
  │                                                               │
  │  cgroups.rs               layer.rs                            │
  │  ┌──────────────────┐     ┌──────────────────┐                │
  │  │ write cgroup     │     │ tar decompress   │                │
  │  │ files            │     │ SHA256 verify    │                │
  │  └──────────────────┘     │ file extraction  │                │
  │                           └──────────────────┘                │
  │                                                               │
  │  pipe reading                                                 │
  │  ┌──────────────────┐                                         │
  │  │ read(fd, buf)    │ → base64 → ContainerOutput              │
  │  └──────────────────┘                                         │
  └───────────────────────────────────────────────────────────────┘

  Rule: clone/fork/exec/mount/waitpid MUST NOT run inline
        in async fn — always wrap in spawn_blocking
```

---

## 14. Error Handling Chain

```
  Container init failure (child process):
    execvp fails → _exit(127)
    pivot_root fails → _exit(127)

                    │
                    ▼

  Parent (spawn_blocking):
    waitpid() returns exit_code=127
    SpawnResult { pid, output_reader }

                    │
                    ▼

  Handler (async):
    drain output pipe (may contain error message)
    send ContainerStopped { exit_code: 127 } → client

                    │
                    ▼

  CLI:
    receive ContainerStopped → exit(127)


  Infrastructure failure (overlay/cgroup/pull):
    filesystem.setup_rootfs()
        .context("Failed to create overlay mount")?
                    │
                    ▼
    handle_run_streaming():
        if Err(e) → cleanup overlay, cgroup, state
                  → send DaemonResponse::Error { message: e.to_string() }
                    │
                    ▼
    CLI:
        receive Error → eprintln!(message) → exit(1)


  Cleanup-on-error convention:
  ┌──────────────────────────────────────────────────────────┐
  │ fn create_container() → Result<ContainerId> {            │
  │     let rootfs = create_overlay().context("overlay")?;   │
  │                                                          │
  │     if let Err(e) = setup_cgroup() {                     │
  │         // Best-effort cleanup — warn, don't propagate   │
  │         if let Err(cleanup_err) = destroy_overlay() {    │
  │             warn!(error = %cleanup_err,                  │
  │                   "overlay cleanup failed");             │
  │         }                                                │
  │         return Err(e).context("cgroup");                 │
  │     }                                                    │
  │ }                                                        │
  └──────────────────────────────────────────────────────────┘
```

---

## 15. Runtime Directory Layout

```
  /run/minibox/                          ← MINIBOX_RUN_DIR
  ├── miniboxd.sock                      ← Unix socket (0600)
  └── containers/
      └── {container_id}/
          └── pid                        ← host PID file

  /var/lib/minibox/                      ← MINIBOX_DATA_DIR (root)
  ~/.minibox/cache/                          ← MINIBOX_DATA_DIR (non-root)
  ├── images/
  │   ├── library/
  │   │   ├── alpine/
  │   │   │   └── latest/
  │   │   │       ├── manifest.json
  │   │   │       ├── sha256_abc.../     ← extracted layer 1
  │   │   │       │   ├── bin/
  │   │   │       │   ├── etc/
  │   │   │       │   └── usr/
  │   │   │       └── sha256_def.../     ← extracted layer 2
  │   │   └── nginx/
  │   │       └── ...
  │   └── ghcr.io/
  │       └── org/name/tag/
  ├── containers/
  │   └── {container_id}/
  │       ├── merged/                    ← overlay mount point (rootfs)
  │       ├── upper/                     ← writable layer
  │       └── work/                      ← overlay workdir
  └── state.json                         ← persisted container records

  /sys/fs/cgroup/                        ← MINIBOX_CGROUP_ROOT
  └── minibox.slice/
      └── miniboxd.service/
          ├── supervisor/                ← daemon's own leaf cgroup
          └── {container_id}/            ← per-container cgroup
              ├── cgroup.procs
              ├── memory.max
              ├── cpu.weight
              └── pids.max
```

---

## 16. Tracing Convention

```
  Severity:
  ┌──────────┬────────────────────────────────────────────────────┐
  │ error!   │ Unrecoverable: container init crash, fatal exec    │
  │ warn!    │ Security rejections, degraded, cleanup failures    │
  │ info!    │ Lifecycle: start/stop, pull phases, overlay, pivot │
  │ debug!   │ Syscall args, byte counts, state transitions       │
  └──────────┴────────────────────────────────────────────────────┘

  Message format: "subsystem: verb noun" (lowercase)

  ┌───────────────────────────────────────────────────────────────┐
  │ ✓ CORRECT                                                     │
  │                                                               │
  │   tracing::info!(                                             │
  │       container_id = %id,                                     │
  │       pid = pid.as_raw(),                                     │
  │       rootfs = %config.rootfs.display(),                      │
  │       "container: process started"                            │
  │   );                                                          │
  │                                                               │
  │   tracing::warn!(                                             │
  │       entry = %entry.display(),                               │
  │       target = %symlink_target.display(),                     │
  │       "tar: rejected absolute symlink"                        │
  │   );                                                          │
  └───────────────────────────────────────────────────────────────┘

  ┌───────────────────────────────────────────────────────────────┐
  │ ✗ WRONG                                                       │
  │                                                               │
  │   tracing::info!("Container {} started with PID {}", id, pid);│
  │   //  ↑ values embedded in message — not queryable            │
  └───────────────────────────────────────────────────────────────┘

  Canonical fields:
  ┌───────────────────┬─────────┬────────────────────────────────┐
  │ Field             │ Type    │ Used in                        │
  ├───────────────────┼─────────┼────────────────────────────────┤
  │ container_id      │ &str    │ all container operations       │
  │ pid               │ u32     │ process lifecycle              │
  │ child_pid         │ i32     │ namespace.rs clone result      │
  │ clone_flags       │ i32     │ namespace.rs                   │
  │ entry             │ &Path   │ tar security events            │
  │ kind              │ &Type   │ device node rejection          │
  │ target            │ &Path   │ symlink target (original)      │
  │ rewritten_target  │ &Path   │ symlink target (after rewrite) │
  │ new_root          │ &Path   │ pivot_root destination         │
  │ fds_closed        │ usize   │ close_extra_fds count          │
  │ command           │ &str    │ container entrypoint           │
  │ rootfs            │ &Path   │ container rootfs path          │
  │ mode_before/after │ u32     │ permission bit changes         │
  └───────────────────┴─────────┴────────────────────────────────┘
```

---

## 17. CLI Command Flow

```
  minibox <subcommand>
      │
      ├─ pull <image[:tag]>
      │   └─ DaemonRequest::Pull { image, tag }
      │      → daemon pulls + caches
      │      ← DaemonResponse::Success
      │
      ├─ run <image[:tag]> [-- command args...]
      │   └─ DaemonRequest::Run { image, tag, command, ephemeral: true }
      │      → daemon pulls (if needed) + creates container + streams output
      │      ← ContainerCreated → ContainerOutput* → ContainerStopped
      │      CLI exits with container's exit code
      │
      ├─ ps
      │   └─ DaemonRequest::List
      │      ← DaemonResponse::ContainerList { containers }
      │      CLI prints table: ID, IMAGE, COMMAND, STATE, CREATED, PID
      │
      ├─ stop <id>
      │   └─ DaemonRequest::Stop { id }
      │      → daemon sends SIGTERM, waits, then SIGKILL
      │      ← DaemonResponse::Success
      │
      └─ rm <id>
          └─ DaemonRequest::Remove { id }
             → daemon unmounts overlay, cleans cgroup, removes state
             ← DaemonResponse::Success


  Socket connection pattern:
  ┌──────────────────────────────────────────────────────────┐
  │ let socket_path = env::var("MINIBOX_SOCKET_PATH")        │
  │     .unwrap_or("/run/minibox/miniboxd.sock");            │
  │                                                          │
  │ let stream = UnixStream::connect(socket_path)?;          │
  │ let mut reader = BufReader::new(&stream);                │
  │ let mut writer = BufWriter::new(&stream);                │
  │                                                          │
  │ // Send request                                          │
  │ serde_json::to_writer(&mut writer, &request)?;           │
  │ writer.write_all(b"\n")?;                                │
  │ writer.flush()?;                                         │
  │                                                          │
  │ // Read responses (loop for streaming)                   │
  │ loop {                                                   │
  │     let mut line = String::new();                        │
  │     reader.read_line(&mut line)?;                        │
  │     let response: DaemonResponse = serde_json::from_str? │
  │     match response { ... }                               │
  │ }                                                        │
  └──────────────────────────────────────────────────────────┘
```

---

## 18. Platform Dispatch

```
  miniboxd/src/main.rs

  fn main() {
      #[cfg(target_os = "linux")]
      {
          // Full daemon: adapter selection, DI, socket bind, accept loop
          // → daemonbox::server::run_server()
      }

      #[cfg(target_os = "macos")]
      {
          // Delegate to macbox
          macbox::start();
          // → Colima adapter suite (limactl + nerdctl)
      }

      #[cfg(target_os = "windows")]
      {
          // Delegate to winbox
          winbox::start();
          // → stub (not implemented)
      }
  }

  Platform crate naming convention: {platform}box
  ├─ minibox   (Linux namespaces, cgroups, overlay)
  ├─ macbox     (macOS via Colima VM)
  └─ winbox     (Windows stub)

  Compile gates:
  ├─ minibox/container/  → #[cfg(target_os = "linux")]
  ├─ nix syscall wrappers → #[cfg(unix)]
  └─ miniboxd compiles on all platforms (macbox::start() dispatch)
```

---

## 19. Testing Pyramid

```
  ┌───────────────────────────────────────────────────────────┐
  │                    E2E Tests (14)                         │
  │                 Linux + root only                         │
  │           just test-e2e / cargo xtask test-e2e-suite      │
  │                                                           │
  │  Full daemon + CLI: pull, run, ps, stop, rm               │
  │  Streaming output, exit codes, concurrent containers      │
  └─────────────────────────────┬─────────────────────────────┘
                                │
  ┌─────────────────────────────▼─────────────────────────────┐
  │              Integration Tests (16 + 8)                   │
  │                 Linux + root only                         │
  │           just test-integration                           │
  │                                                           │
  │  Cgroup: memory limits, CPU weight, pids.max, io.max      │
  │  Overlay: mount, layer stacking, cleanup                  │
  │  Container: namespace isolation, pivot_root               │
  └─────────────────────────────┬─────────────────────────────┘
                                │
  ┌─────────────────────────────▼─────────────────────────────┐
  │             Property Tests (33)                           │
  │                Any platform                               │
  │           cargo xtask test-property                       │
  │                                                           │
  │  Proptest: protocol roundtrip, state machine, image ref   │
  │  DaemonState invariants, handler edge cases               │
  └─────────────────────────────┬─────────────────────────────┘
                                │
  ┌─────────────────────────────▼─────────────────────────────┐
  │           Unit + Conformance Tests (257)                  │
  │              Any platform (4 skipped on macOS)            │
  │           cargo xtask test-unit / just test-unit          │
  │                                                           │
  │  155 lib + 11 CLI + 22 handler + 16 conformance           │
  │  + 13 minibox-llm + 36 minibox-secrets + 4 macOS-skipped  │
  │                                                           │
  │  Mock adapters from minibox_core::adapters::mocks         │
  └───────────────────────────────────────────────────────────┘

  macOS quality gates:
    cargo fmt --all --check
    cargo clippy (all platform crates) -- -D warnings
    cargo xtask test-unit
```
