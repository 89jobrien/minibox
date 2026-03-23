# Local Image Store + ghcr.io Adapter + Protocol Streaming Design

**Date:** 2026-03-19
**Status:** Draft

## Overview

Three self-contained additions that form the foundation for minibox-native CI execution:

1. **`~/.mbx/cache/`** — user-local image store. Image pulls write to the user's home directory; no root required for image storage. Runtime state (containers, mounts, cgroups) is unchanged.
2. **`GhcrRegistry`** — a new `ImageRegistry` adapter for GitHub Container Registry (`ghcr.io`). Nearly identical to `DockerHubRegistry` — same OCI Distribution Spec — with GitHub PAT auth instead of Docker Hub anonymous token exchange.
3. **Protocol streaming** — `ContainerOutput` and `ContainerStopped` message types that enable stdout/stderr to be piped from a running container back to the CLI. Required before containerized hook execution can work.

These are the implementation prerequisites for the containerized CI execution phases (see containerized-ci-execution-design.md).

---

## Image Reference Format

### Canonical format

```
[REGISTRY/]NAMESPACE/NAME[:TAG]
```

| Component  | Default        | Example               |
| ---------- | -------------- | --------------------- |
| `REGISTRY` | `docker.io`    | `ghcr.io`             |
| `NAMESPACE`| `library`      | `org` or `library`    |
| `NAME`     | (required)     | `alpine`, `minibox-rust-ci` |
| `TAG`      | `latest`       | `stable`, `1.88`      |

### Parsing rules

- `alpine` → `docker.io/library/alpine:latest`
- `ubuntu:22.04` → `docker.io/library/ubuntu:22.04`
- `myorg/myimage` → `docker.io/myorg/myimage:latest`
- `ghcr.io/org/minibox-rust-ci:stable` → `ghcr.io/org/minibox-rust-ci:stable`

A reference containing a `.` or `:` in the first path component is treated as a registry hostname. All other references default to `docker.io`.

### Implementation

A new `ImageRef` type in `crates/linuxbox/src/image/reference.rs`:

```rust
pub struct ImageRef {
    pub registry: String,    // e.g. "docker.io", "ghcr.io"
    pub namespace: String,   // e.g. "library", "org"
    pub name: String,        // e.g. "alpine"
    pub tag: String,         // e.g. "latest"
}

impl ImageRef {
    pub fn parse(s: &str) -> Result<Self, ImageRefError> { ... }
    pub fn registry_host(&self) -> &str { ... }  // "registry-1.docker.io" for docker.io
    pub fn repository(&self) -> String { ... }   // "library/alpine"
    pub fn cache_path(&self, base: &Path) -> PathBuf { ... }
}
```

The `cache_path` method produces the path under `MINIBOX_DATA_DIR/images/` where the image is stored, e.g. `docker.io/library/alpine/latest/`.

---

## `~/.mbx/cache/` — User-Local Image Store

### Motivation

Today `MINIBOX_DATA_DIR` defaults to `/var/lib/minibox/` — a system path requiring root. Image pulling is pure HTTP + file writes and does not need root. Separating the image cache from the runtime state allows image pulls without privilege escalation.

### Directory Structure

```
~/.mbx/cache/
  images/
    <registry>/
      <namespace>/<name>/
        <tag>/
          manifest.json
          layers/
            <digest>/           # extracted layer contents
  cargo-registry/               # shared Cargo registry cache; created in Phase 2,
                                # bind-mounted into containers at /root/.cargo/registry
```

Runtime state remains at:

```
/var/lib/minibox/containers/    # overlay upper/work dirs (root-owned)
/run/minibox/                   # socket, pid files (root-owned)
```

### Configuration

`MINIBOX_DATA_DIR` controls image storage. The daemon resolves this at startup based on its own effective UID:

1. `MINIBOX_DATA_DIR` env var (explicit override — unchanged)
2. `~/.mbx/cache/` if effective UID is non-root
3. `/var/lib/minibox/` if effective UID is root (unchanged)

No breaking change: existing deployments using `sudo miniboxd` continue to use `/var/lib/minibox/`.

**Non-root image pulls:** When a future CLI-direct pull path is implemented (without daemon), the CLI applies the same resolution: `MINIBOX_DATA_DIR` → `~/.mbx/cache/`. For now, all pulls go through the daemon which runs as root and uses `/var/lib/minibox/` unless `MINIBOX_DATA_DIR` is explicitly overridden.

### Image path encoding

Registry hostname and full namespace are included in the path:

```
~/.mbx/cache/images/docker.io/library/alpine/latest/
~/.mbx/cache/images/ghcr.io/org/minibox-rust-ci/stable/
```

### Affected modules

- `crates/linuxbox/src/image/reference.rs` — new `ImageRef` type (see above)
- `crates/linuxbox/src/adapters/registry.rs` — `DockerHubRegistry`: use `ImageRef::cache_path()` for layer extraction paths; update `data_dir` resolution logic
- `crates/linuxbox/src/image/` — layer extraction paths use resolved `data_dir`
- `crates/linuxbox/src/preflight.rs` — doctor check reports active data dir

---

## `GhcrRegistry` — GitHub Container Registry Adapter

### Motivation

Pre-baked CI images will be published to `ghcr.io`. Minibox needs to pull from ghcr.io to use them locally and in self-hosted GHA jobs.

### OCI Distribution Spec compatibility

`ghcr.io` implements the OCI Distribution Spec (same as Docker Hub v2). The only differences from `DockerHubRegistry` are:

|                     | DockerHubRegistry           | GhcrRegistry                    |
| ------------------- | --------------------------- | ------------------------------- |
| Registry base URL   | `registry-1.docker.io`      | `ghcr.io`                       |
| Auth endpoint       | `auth.docker.io/token`      | Parsed from `WWW-Authenticate`  |
| Anonymous pulls     | Yes (public images)         | Yes (public images)             |
| Authenticated pulls | Not implemented             | `GHCR_TOKEN` env var            |
| Token scope         | `repository:pull`           | `repository:read`               |

### Auth flow

```
1. Attempt unauthenticated manifest request.
2. If 200 OK: use response directly (public image, no auth needed).
3. If 401: parse WWW-Authenticate response header:
     Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:ORG/IMAGE:pull"
4. Build token request URL: realm?service=SERVICE&scope=SCOPE
5. If GHCR_TOKEN is set:
     GET <token-url> with Authorization: Bearer <GHCR_TOKEN>
6. If GHCR_TOKEN is not set:
     GET <token-url> without Authorization (anonymous; works for public images)
7. Parse JSON response: { "token": "eyJ..." }
8. Use token as Bearer on all subsequent manifest and blob requests.
```

`GHCR_TOKEN`: GitHub PAT with `read:packages` scope (local dev). In GHA: `GITHUB_TOKEN` is set automatically and has `read:packages` on the repo's packages.

### Registry selection mechanism

The daemon maintains one instance of each registry adapter at startup. When a `Pull` or `Run` request arrives with an image reference, the handler calls `select_registry(image_ref)` which routes to the appropriate adapter based on registry hostname:

```rust
// crates/miniboxd/src/handler.rs
fn select_registry<'a>(
    image_ref: &ImageRef,
    docker: &'a DockerHubRegistry,
    ghcr: &'a GhcrRegistry,
) -> &'a dyn ImageRegistry {
    match image_ref.registry.as_str() {
        "ghcr.io" => ghcr,
        _         => docker,   // docker.io or any unknown registry → Docker Hub path
    }
}
```

Both `DockerHubRegistry` and `GhcrRegistry` are initialized unconditionally at daemon startup. `MINIBOX_ADAPTER` continues to control the container runtime adapter (`native`, `gke`) — registry selection is now per-request and based on image reference, not `MINIBOX_ADAPTER`.

### Implementation

New file: `crates/linuxbox/src/adapters/ghcr.rs`

```rust
pub struct GhcrRegistry {
    data_dir: PathBuf,
    token: Option<String>,      // GHCR_TOKEN env var
    http: reqwest::Client,
}
```

Implements `ImageRegistry` trait. Shares layer extraction, manifest parsing, and digest verification with `DockerHubRegistry` — these live in `crates/linuxbox/src/image/` and are registry-agnostic.

### Affected modules

- `crates/linuxbox/src/adapters/ghcr.rs` — new file
- `crates/linuxbox/src/adapters/mod.rs` — export `GhcrRegistry`
- `crates/miniboxd/src/main.rs` — initialize both registries at startup
- `crates/miniboxd/src/handler.rs` — add `select_registry()`, update all image operations to call it

---

## Protocol Streaming — ContainerOutput + ContainerStopped

### Motivation

Currently the daemon discards container stdout/stderr. To run CI tools in containers and see their output, the daemon must stream stdout/stderr back to the client over the Unix socket.

### RunContainer extension

`RunContainer` gains an `ephemeral` flag (also Phase 1 scope):

```rust
pub struct RunContainer {
    pub image: ImageRef,
    pub command: Vec<String>,
    pub ephemeral: bool,   // if true, daemon removes container on exit
    // Phase 2 addition (not in scope here):
    // pub mounts: Vec<Mount>,
}
```

When `ephemeral: true`, the daemon deletes the container's overlay upper dir and state entry after `ContainerStopped` is sent. No separate `remove` call required from the client. All containerized CI invocations use `ephemeral: true`.

### New message types

Added to `crates/linuxbox/src/protocol.rs`:

```rust
#[serde(tag = "type")]
pub enum DaemonResponse {
    // ... existing variants unchanged ...

    /// A chunk of output from a running container. Sent zero or more times
    /// before ContainerStopped. `data` is base64-encoded raw bytes.
    ContainerOutput {
        stream: OutputStreamKind,
        data: String,
    },

    /// Terminal message for a streaming run. Sent exactly once when the
    /// container process exits. Signals end of ContainerOutput stream.
    ContainerStopped {
        exit_code: i32,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputStreamKind {
    Stdout,
    Stderr,
}
```

### Protocol flow

```
Client                              Daemon
  |---> RunContainer { image, cmd } --->|
  |                                     | (container starts)
  |<--- ContainerOutput { stdout, ... }--|  (zero or more)
  |<--- ContainerOutput { stderr, ... }--|  (interleaved)
  |<--- ContainerStopped { exit_code } --|  (exactly one, terminal)
```

All messages are newline-delimited JSON over the existing Unix socket, consistent with the existing protocol framing. The client reads until `ContainerStopped`, writes decoded chunks to its own stdout/stderr as they arrive, and exits with the received `exit_code`.

### Implementation

- `crates/linuxbox/src/container/process.rs` — pipe child stdout/stderr to a pair of `UnixStream` handles; spawn two reader tasks that send `ContainerOutput` messages to the client connection
- `crates/miniboxd/src/handler.rs` — `handle_run`: after spawning container, enter read loop sending `ContainerOutput` until process exits, then send `ContainerStopped`
- `crates/minibox-cli/src/commands/run.rs` — read streaming response, write to stdout/stderr, exit with received code

### Backward compatibility

Existing clients that only read the first response message (e.g. expecting `ContainerCreated`) will ignore subsequent messages. No client breaks. The streaming behavior is only activated when the daemon has stdout/stderr handles to forward; if not connected to a process, no `ContainerOutput` messages are sent.

---

## Security

- `GHCR_TOKEN` is never logged (existing tracing conventions: structured fields only, no secret values)
- Path validation for `~/.mbx/cache/` follows same `canonicalize()` + `..` rejection as `/var/lib/minibox/`
- Layer size limits (1 GB per layer, 5 GB total) apply regardless of registry

---

## Testing

- Unit tests: `GhcrRegistry` with mock HTTP server (same pattern as `DockerHubRegistry` tests)
- `has_image` / `get_image_layers` with cached and uncached states
- Auth flow: with and without `GHCR_TOKEN`; public image anonymous success
- `ImageRef::parse()`: full matrix of input formats
- `select_registry()`: ghcr.io routes to GhcrRegistry, others to DockerHubRegistry
- `ContainerOutput` / `ContainerStopped` serialization round-trip
- `MINIBOX_DATA_DIR` resolution: non-root default, root default, env override
- Integration test: `minibox pull ghcr.io/org/minibox-rust-ci:stable` on Linux (requires network, tagged `#[ignore]`)

---

## Success Criteria

- `minibox pull ghcr.io/<org>/<image>:<tag>` works without `MINIBOX_ADAPTER` change
- Pulled layers land in `MINIBOX_DATA_DIR/images/ghcr.io/...`
- `GHCR_TOKEN` unset → anonymous pull succeeds for public images
- Root `miniboxd` uses `/var/lib/minibox/` (no regression)
- `minibox run <container>` streams stdout/stderr to terminal and exits with container's exit code
