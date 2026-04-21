# Docker Parity Roadmap: Exec, Push, Commit, Build

**Date:** 2026-04-01
**Status:** Draft
**Goal:** Replace Docker as a runtime dependency in maestro by extending minibox with the four missing capabilities that stand between dockerbox and full Docker API coverage.

---

## Motivation

Maestro currently depends on Docker (via the `bollard` Rust crate) for its container workflow. The dockerbox shim (HTTP-over-Unix-socket Docker API translator) today covers ~35% of maestro's Docker API surface. The four missing capabilities are:

| Capability           | Maestro usage                                  |
| -------------------- | ---------------------------------------------- |
| **Exec**             | Run commands inside running session containers |
| **Image Push**       | Push built images to GHCR / GCR                |
| **Container Commit** | Snapshot containers after prebuild steps       |
| **Image Build**      | Build session images from Dockerfiles          |

These four capabilities, plus the existing dockerbox shim, will allow maestro to remove its Docker dependency entirely.

---

## Architecture: New Domain Traits (ISP)

New capabilities are added as **new focused traits**, not extensions to `ContainerRuntime` or `ImageRegistry`. This preserves the existing adapter surface — adapters that don't support exec (e.g. `ColimaRuntime`) are not forced to implement it.

### New traits in `minibox-core/src/domain.rs`

```rust
/// Port for executing commands inside running containers.
/// Implemented by: NativeExecRuntime (Linux nsenter)
#[async_trait]
pub trait ExecRuntime: AsAny + Send + Sync {
    async fn exec(
        &self,
        container_id: &ContainerId,
        config: &ExecConfig,
    ) -> Result<ExecHandle>;
}

/// Port for pushing images to OCI-compliant registries.
/// Implemented by: OciPushAdapter
#[async_trait]
pub trait ImagePusher: AsAny + Send + Sync {
    async fn push_image(
        &self,
        image_ref: &ImageRef,
        credentials: &RegistryCredentials,
    ) -> Result<PushResult>;
}

/// Port for committing a running container's filesystem diff as a new image.
/// Implemented by: OverlayCommitAdapter (Linux overlay upperdir snapshot)
#[async_trait]
pub trait ContainerCommitter: AsAny + Send + Sync {
    async fn commit(
        &self,
        container_id: &ContainerId,
        target: &ImageRef,
        config: &CommitConfig,
    ) -> Result<ImageMetadata>;
}

/// Port for building container images from Dockerfiles.
/// Implemented by: MiniboxImageBuilder (basic Dockerfile subset)
#[async_trait]
pub trait ImageBuilder: AsAny + Send + Sync {
    async fn build_image(
        &self,
        context: &BuildContext,
        config: &BuildConfig,
    ) -> Result<ImageMetadata>;
}
```

### New domain types

```rust
pub struct ExecConfig {
    pub cmd: Vec<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<PathBuf>,
    pub tty: bool,
}

pub struct ExecHandle {
    pub id: String,  // exec instance ID
    // stdout/stderr delivered via DaemonResponse::ContainerOutput stream
}

pub enum RegistryCredentials {
    Anonymous,
    Basic { username: String, password: String },
    Token(String),
}

pub struct PushResult {
    pub digest: String,
    pub size_bytes: u64,
}

pub struct CommitConfig {
    pub author: Option<String>,
    pub message: Option<String>,
    pub env: Vec<(String, String)>,
    pub cmd: Option<Vec<String>>,
}

pub struct BuildContext {
    pub directory: PathBuf,
    pub dockerfile: PathBuf,  // relative to directory, default "Dockerfile"
}

pub struct BuildConfig {
    pub tag: ImageRef,
    pub build_args: Vec<(String, String)>,
    pub no_cache: bool,
    pub platform: Option<String>,  // e.g. "linux/amd64"
}
```

---

## Phase 1: Exec

### Protocol changes (both protocol.rs files)

```rust
// New DaemonRequest variants
Exec {
    container_id: String,
    cmd: Vec<String>,
    env: Vec<(String, String)>,
    working_dir: Option<String>,
    tty: bool,
},

// New DaemonResponse variants
ExecStarted {
    exec_id: String,
},
// ContainerOutput and ContainerStopped are reused for exec output + exit code
```

### NativeExecRuntime adapter (`crates/minibox/src/adapters/exec.rs`)

Uses Linux namespace joining via `/proc/{pid}/ns/*`:

1. Look up container PID from `DaemonState`
2. Open namespace fds: `mnt`, `pid`, `net`, `uts`, `ipc` from `/proc/{pid}/ns/`
3. `tokio::task::spawn_blocking` — fork a child process
4. In child: `setns()` for each namespace fd, then `execve()` the requested command
5. Stream stdout/stderr back via `ContainerOutput` protocol messages
6. Send `ContainerStopped` with exit code when process exits

**Security:** `SO_PEERCRED` UID==0 check already guards the socket — exec inherits this gate. Container PID is looked up from trusted `DaemonState`, never from client input.

**State dependency:** `ContainerRecord` gains `pid: Option<u32>` field.

### dockerbox wiring

```
POST /containers/{id}/exec  -> create ExecInstance, return {"Id": exec_id}
POST /exec/{id}/start       -> send DaemonRequest::Exec, stream ContainerOutput
GET  /exec/{id}/json        -> return ExecInstance status
```

### Testing

- Unit: mock `ExecRuntime`, verify nsenter config generation
- Integration: Linux+root, exec `/bin/sh -c "echo hello"` into running alpine container
- E2E: dockerbox endpoint test via bollard `create_exec` + `start_exec`

---

## Phase 2: Image Push

### OCI Distribution Spec push flow

1. **Check existing blobs** — `HEAD /v2/{name}/blobs/{digest}` — skip upload if already present
2. **Initiate upload** — `POST /v2/{name}/blobs/uploads/`
3. **Upload blob** — `PUT /v2/{name}/blobs/uploads/{uuid}?digest={digest}` with layer tarball body
4. **Upload config blob** — same for image config JSON
5. **Put manifest** — `PUT /v2/{name}/manifests/{tag}` with OCI manifest JSON

### OciPushAdapter (`crates/minibox/src/adapters/push.rs`)

Extends existing `RegistryClient` with push methods:

```rust
impl RegistryClient {
    async fn push_blob(&self, repo: &str, digest: &str, data: Bytes, token: &str) -> Result<()>;
    async fn push_manifest(&self, repo: &str, reference: &str, manifest: &OciManifest, token: &str) -> Result<String>;
    async fn get_push_token(&self, repo: &str, credentials: &RegistryCredentials) -> Result<String>;
}
```

Push tokens require `push` scope: `scope=repository:{repo}:push,pull` in the `www-authenticate` challenge.

### Protocol changes

```rust
Push {
    image_ref: String,
    credentials: PushCredentials,
},

PushProgress {
    layer_digest: String,
    bytes_uploaded: u64,
    total_bytes: u64,
},
// PushComplete reuses Success { message }
```

### dockerbox wiring

```
POST /images/{name}/push  -> DaemonRequest::Push, stream PushProgress
POST /images/{name}/tag   -> local ImageStore rename (no upstream call)
```

### Testing

- Unit: mock `ImagePusher`, verify OCI manifest construction from local image store
- Integration: push to a local OCI registry (`registry:2` via minibox)
- E2E: round-trip pull -> push -> pull from local registry

---

## Phase 3: Container Commit

### Overlay snapshot approach

A container's writable changes live entirely in its `upperdir`. Commit:

1. Tar the `upperdir` — this is the diff layer
2. Compute SHA256 digest of the tarball
3. Write tarball as a new layer blob in `ImageStore`
4. Build new OCI manifest: parent image layers + new diff layer
5. Write new image config JSON (merge parent config + `CommitConfig` overrides)
6. Store under the target `ImageRef` in `ImageStore`

The result is immediately available to `DaemonRequest::Run` without any push.

### OverlayCommitAdapter (`crates/minibox/src/adapters/commit.rs`)

```rust
pub struct OverlayCommitAdapter {
    image_store: Arc<ImageStore>,
    data_dir: PathBuf,
}
```

Steps: locate `data_dir/containers/{id}/upper/` -> `spawn_blocking` tar -> compute digest -> write blob -> build manifest -> store.

**State dependency:** `ContainerRecord` gains `image_ref: String` and `overlay_paths: OverlayPaths`.

### Protocol changes

```rust
Commit {
    container_id: String,
    target_image: String,
    author: Option<String>,
    message: Option<String>,
    env_overrides: Vec<(String, String)>,
    cmd_override: Option<Vec<String>>,
},
// Response: Success { message: digest }
```

### dockerbox wiring

```
POST /containers/{id}/commit  -> DaemonRequest::Commit
```

### Testing

- Unit: mock overlay paths, verify manifest construction and layer digest
- Integration: Linux+root, run alpine -> create file -> commit -> run new image -> verify file present
- E2E: dockerbox `commit_container` endpoint test

---

## Phase 4: Image Build (Basic Dockerfile Parser)

### Supported instruction subset (~90% of real Dockerfiles)

| Instruction           | Semantics                                                    |
| --------------------- | ------------------------------------------------------------ |
| `FROM image[:tag]`    | Base image; must be first non-comment instruction            |
| `FROM image AS name`  | Named stage (multi-stage: only final stage built)            |
| `RUN cmd`             | Execute in container, commit layer                           |
| `COPY src... dest`    | Copy files from build context into image                     |
| `ADD src... dest`     | Like COPY; also handles URLs and tar auto-extract            |
| `ENV key=value`       | Set environment variable                                     |
| `ARG name[=default]`  | Build-time variable (substituted in subsequent instructions) |
| `WORKDIR /path`       | Set working directory                                        |
| `CMD ["cmd", "arg"]`  | Default command (exec form and shell form)                   |
| `ENTRYPOINT ["cmd"]`  | Container entrypoint                                         |
| `EXPOSE port[/proto]` | Document exposed ports (metadata only)                       |
| `LABEL key=value`     | Image metadata                                               |
| `USER name[:group]`   | Set user for RUN/CMD/ENTRYPOINT                              |

**Not supported (deferred):** `HEALTHCHECK`, `VOLUME`, `ONBUILD`, `SHELL`, `STOPSIGNAL`, BuildKit `--mount` syntax, `.dockerignore`.

### MiniboxImageBuilder architecture

```
BuildContext (directory + Dockerfile)
        |
        v
DockerfileParser         -> Vec<Instruction>
        |
        v
BuildPlanner             -> BuildPlan (ordered steps, layer cache keys)
        |
        v
BuildExecutor            -> executes each step:
  +-- RUN  -> start ephemeral container, exec command, commit layer
  +-- COPY -> validate paths, tar files, inject into overlay
  +-- ADD  -> COPY + URL fetch + tar auto-extract
  +-- ENV/WORKDIR/CMD/etc -> config mutations (no container spawn)
        |
        v
ImageStore.write_image() -> final OCI manifest + config
```

`MiniboxImageBuilder` depends on `ExecRuntime` (for `RUN` steps) and `ContainerCommitter` (for layer snapshots). Build naturally composes Phase 1 and Phase 3.

### Layer caching

Cache key: `hash(parent_digest + instruction_text + context_files_digest)`. Hit -> reuse existing blob. Miss -> execute and write new blob. Cache stored in `ImageStore`'s `build_cache/` subdirectory.

### `DockerfileParser` (`crates/minibox/src/image/dockerfile.rs`)

```rust
pub fn parse(input: &str) -> Result<Vec<Instruction>, ParseError>;

pub enum Instruction {
    From { image: String, tag: String, alias: Option<String> },
    Run(ShellOrExec),
    Copy { srcs: Vec<PathBuf>, dest: PathBuf },
    Add { srcs: Vec<AddSource>, dest: PathBuf },
    Env(Vec<(String, String)>),
    Arg { name: String, default: Option<String> },
    Workdir(PathBuf),
    Cmd(ShellOrExec),
    Entrypoint(ShellOrExec),
    Expose { port: u16, proto: Proto },
    Label(Vec<(String, String)>),
    User { name: String, group: Option<String> },
    Comment(String),
}

pub enum ShellOrExec {
    Shell(String),      // RUN cmd -> /bin/sh -c "cmd"
    Exec(Vec<String>),  // RUN ["cmd", "--flag"]
}

pub enum AddSource {
    Local(PathBuf),
    Url(String),
}
```

### Protocol changes

```rust
Build {
    context_tar: Vec<u8>,  // tarball of build context (limit: 2GB)
    dockerfile: String,    // Dockerfile content
    tag: String,
    build_args: Vec<(String, String)>,
    no_cache: bool,
},

BuildOutput {
    step: u32,
    total_steps: u32,
    message: String,
    stream: BuildStream,
},

BuildComplete {
    image_id: String,
    digest: String,
},

pub enum BuildStream { Stdout, Stderr, Status }
```

### dockerbox wiring

```
POST /build  -> DaemonRequest::Build, stream BuildOutput lines
             -> BuildComplete -> HTTP 200 {"stream": "Successfully built {id}\n"}
```

### Testing

- Unit: `DockerfileParser` round-trips for each supported instruction + error cases
- Unit: `BuildPlanner` generates correct step ordering and cache keys
- Integration: Linux+root, build 3-layer alpine-based image, verify final image runs
- E2E: dockerbox `build_image` -> `create_container` -> `start_container` flow

---

## Adapter Wiring (`miniboxd/src/main.rs`)

```rust
// In AdapterSuite::Native
let exec_runtime = Arc::new(NativeExecRuntime::new(Arc::clone(&state)));
let image_pusher = Arc::new(OciPushAdapter::new(Arc::clone(&registry_client)));
let committer = Arc::new(OverlayCommitAdapter::new(Arc::clone(&image_store), data_dir.clone()));
let builder = Arc::new(MiniboxImageBuilder::new(
    Arc::clone(&image_store),
    Arc::clone(&exec_runtime),    // RUN steps
    Arc::clone(&committer),       // layer snapshots
));
```

---

## Phased Delivery

| Phase | Deliverable                                            | Unblocks in maestro                      |
| ----- | ------------------------------------------------------ | ---------------------------------------- |
| **1** | Exec (nsenter + protocol + dockerbox)                  | `exec_command`                           |
| **2** | Image Push (OCI dist spec + dockerbox)                 | `push_image`                             |
| **3** | Container Commit (overlay snapshot + dockerbox)        | `commit_container`                       |
| **4** | Image Build (Dockerfile parser + executor + dockerbox) | `build_image` — final Docker dep removal |

Each phase is independently shippable. Maestro can switch one operation at a time.

---

## Error Handling

New variants in `minibox-core/src/error.rs`:

```rust
// Exec
ExecNotFound { exec_id: String },
ContainerNotRunning { container_id: String },
NsenterFailed { container_id: String, reason: String },

// Push
RegistryAuthFailed { registry: String },
BlobUploadFailed { digest: String, reason: String },
ManifestPushFailed { reason: String },

// Commit
OverlayUpperdirMissing { container_id: String },
LayerTarFailed { reason: String },

// Build
DockerfileNotFound { path: PathBuf },
ParseError { line: u32, instruction: String, reason: String },
UnsupportedInstruction { instruction: String },
BuildStepFailed { step: u32, instruction: String, exit_code: i32 },
ContextTooLarge { size_bytes: u64, limit_bytes: u64 },
```

---

## Security Considerations

- **Exec:** Inherits `SO_PEERCRED` UID==0 gate. Container PID looked up from `DaemonState` only — never from client-supplied raw PIDs.
- **Push:** Credentials never logged (tracing redacts `password` fields). Push tokens scoped to specific repository.
- **Commit:** `upperdir` path derived from `DaemonState`, not client input. No path traversal risk.
- **Build:** Build context tar extracted with `validate_layer_path()` (same as image pulls). `RUN` commands execute inside isolated container namespaces. Context size limited to 2GB.

---

## Tracing

```rust
// Exec
tracing::info!(container_id = %id, exec_id = %exec_id, cmd = ?config.cmd, "exec: process started");
tracing::info!(container_id = %id, exec_id = %exec_id, exit_code = code, "exec: process exited");

// Push
tracing::info!(image_ref = %image_ref, registry = %registry, "push: started");
tracing::info!(digest = %digest, bytes = size, "push: blob uploaded");
tracing::info!(image_ref = %image_ref, digest = %manifest_digest, "push: complete");

// Commit
tracing::info!(container_id = %id, target = %target, "commit: snapshot started");
tracing::info!(target = %target, digest = %digest, layers = layer_count, "commit: complete");

// Build
tracing::info!(tag = %tag, steps = total, "build: started");
tracing::info!(step = n, instruction = %instr, "build: step started");
tracing::info!(tag = %tag, image_id = %id, "build: complete");
```

---

## Files Changed (summary)

| File                              | Change                                                    |
| --------------------------------- | --------------------------------------------------------- |
| `minibox-core/src/domain.rs`      | Add 4 new traits + new domain types                       |
| `minibox-core/src/error.rs`       | Add new DomainError variants                              |
| `minibox-core/src/protocol.rs`    | Add Exec/Push/Commit/Build request+response variants      |
| `minibox/src/protocol.rs`         | Mirror protocol changes                                   |
| `minibox/src/adapters/exec.rs`    | New — NativeExecRuntime                                   |
| `minibox/src/adapters/push.rs`    | New — OciPushAdapter                                      |
| `minibox/src/adapters/commit.rs`  | New — OverlayCommitAdapter                                |
| `minibox/src/image/dockerfile.rs` | New — DockerfileParser + BuildPlanner                     |
| `minibox/src/adapters/builder.rs` | New — MiniboxImageBuilder                                 |
| `daemonbox/src/handler.rs`        | Add handle_exec, handle_push, handle_commit, handle_build |
| `daemonbox/src/state.rs`          | Add pid + image_ref + overlay_paths to ContainerRecord    |
| `miniboxd/src/main.rs`            | Wire new adapters into native suite                       |
| `dockerbox/src/api/`              | Add exec, push, commit, build endpoints                   |
| `dockerbox/src/domain/`           | Extend ContainerRuntime trait                             |
