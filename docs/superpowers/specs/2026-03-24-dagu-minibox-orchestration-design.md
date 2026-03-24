# Dagu + Minibox Orchestration Integration Design

**Date:** 2026-03-24
**Status:** Under Review

---

## Overview

Integrate dagu (workflow orchestration engine) with minibox (container runtime) to enable complex multi-container job workflows. This design proposes a phased approach: run dagu as a containerized service, then add orchestration capabilities through a Go plugin that communicates with a Rust HTTP controller.

**Goals:**
- Enable dagu DAG workflows to orchestrate minibox containers
- Provide real-time output streaming and resource limit control
- Work across platforms (macOS via colima, Linux native)
- Keep concerns separated (dagu handles workflows, minibox handles containers, controller bridges them)

**Scope:**
- Phase 1: Containerized dagu (approach 1)
- Phase 2: minibox-client library extraction
- Phase 3: mbxctl HTTP controller (approach 4 hybrid)
- Phase 4: mbx-dagu Go plugin (approach 2 integration)

---

## Architecture

### High-Level Flow

```
┌────────────────────────────────────────────────────────────┐
│  Dagu Container (ghcr.io/dagu-org/dagu:minibox)           │
│  ┌────────────────────────────────────────────────────────┐
│  │  DAG Definition (YAML)                                 │
│  │  steps:                                                │
│  │    - name: build                                       │
│  │      executor: minibox                                 │
│  │      command: alpine                                   │
│  │      args: [npm, build]                               │
│  │                                                        │
│  │  ┌──────────────────────────────────────────────────┐ │
│  │  │ mbx-dagu Plugin (Go)                             │ │
│  │  │ - Implements dagu Executor interface             │ │
│  │  │ - Translates DAG steps → HTTP calls              │ │
│  │  │ - Reads MINIBOX_CONTROLLER env var               │ │
│  │  └──────────────────────────────────────────────────┘ │
│  └────────────────────────────────────────────────────────┘
│                       ↓ HTTP
│  ┌────────────────────────────────────────────────────────┐
│  │  mbxctl (localhost:9999) — Rust Controller             │
│  │  ┌────────────────────────────────────────────────────┐
│  │  │  HTTP API:                                         │
│  │  │  POST   /api/v1/jobs                              │
│  │  │  GET    /api/v1/jobs/{id}                         │
│  │  │  DELETE /api/v1/jobs/{id}                         │
│  │  │  GET    /api/v1/jobs/{id}/logs (stream)           │
│  │  └────────────────────────────────────────────────────┘
│  │                       ↓ Unix Socket
│  │  ┌────────────────────────────────────────────────────┐
│  │  │  minibox-client Library                            │
│  │  │  - Parses MINIBOX_SOCKET env var                  │
│  │  │  - Sends DaemonRequest to miniboxd                │
│  │  │  - Receives & forwards DaemonResponse             │
│  │  └────────────────────────────────────────────────────┘
│  └────────────────────────────────────────────────────────┘
│                       ↓ Unix Socket
│  ┌────────────────────────────────────────────────────────┐
│  │  Miniboxd (daemon)                                     │
│  │  - Creates & manages containers                        │
│  │  - Streams output via ephemeral protocol               │
│  └────────────────────────────────────────────────────────┘
```

### Components

| Component | Language | Repository | Purpose |
|-----------|----------|------------|---------|
| **dagu:minibox** | Docker | ghcr.io | Container image with dagu + mbx-dagu plugin |
| **minibox-client** | Rust | minibox/crates | Client library for Unix socket communication |
| **mbxctl** | Rust | separate (mbxctl) | HTTP server wrapping minibox-client |
| **mbx-dagu** | Go | separate (mbx-dagu) | Dagu executor plugin calling mbxctl |

---

## Phase 1: Dagu Container Image

### Objective

Build and publish a container image with dagu pre-installed and the mbx-dagu plugin configured. Users can run it in minibox and access the Web UI immediately.

### Implementation

**Dockerfile** (in `mbx-dagu` repo):

```dockerfile
# Multi-stage: build Go plugin, then add to dagu image

FROM golang:1.23-alpine AS plugin-builder
WORKDIR /src/mbx-dagu
COPY . .
RUN go build -o /tmp/mbx-dagu-plugin ./cmd/plugin

FROM ghcr.io/dagu-org/dagu:latest
RUN apk add --no-cache ca-certificates

# Copy compiled plugin
COPY --from=plugin-builder /tmp/mbx-dagu-plugin /usr/local/bin/

# Dagu configuration to register the plugin
COPY dagu-config.yaml /etc/dagu/

EXPOSE 8080

ENTRYPOINT ["/dagu", "server", "--host", "0.0.0.0", "--port", "8080"]
```

**dagu-config.yaml**:

```yaml
executors:
  minibox:
    binary: /usr/local/bin/mbx-dagu-plugin
    env:
      MINIBOX_CONTROLLER: http://localhost:9999
```

### Usage

```bash
minibox run dagu:minibox \
  -e MINIBOX_CONTROLLER=http://localhost:9999 \
  -v /run/minibox/miniboxd.sock:/run/minibox/miniboxd.sock:ro \
  -- /dagu server

# Dagu Web UI available at http://localhost:8080
```

### Delivery

- Automated Docker build in `mbx-dagu` CI
- Published to `ghcr.io/dagu-org/dagu:minibox` (or user's registry)
- Build triggered on every release of mbx-dagu

---

## Phase 2: minibox-client Library

### Objective

Extract socket communication logic from minibox-cli into a reusable library. Both minibox-cli and mbxctl can use it, avoiding duplication and enabling consistent error handling.

### API Design

```rust
// crates/minibox-client/src/lib.rs

pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonClient {
    /// Create a new client, reading socket path from MINIBOX_SOCKET env var or default.
    pub fn new() -> Result<Self>;

    /// Send a request and return a response stream.
    pub async fn call(
        &self,
        request: DaemonRequest,
    ) -> Result<DaemonResponseStream>;
}

/// Stream of responses from the daemon.
pub struct DaemonResponseStream {
    reader: BufReader<OwnedReadHalf>,
}

impl DaemonResponseStream {
    /// Get the next response (blocks until one arrives).
    pub async fn next(&mut self) -> Result<Option<DaemonResponse>>;

    /// Collect all remaining responses into a vector.
    pub async fn try_collect(self) -> Result<Vec<DaemonResponse>>;
}

impl futures::Stream for DaemonResponseStream {
    type Item = Result<DaemonResponse>;
    // Standard Stream trait for composability
}
```

### Socket Path Resolution

The client reads from `MINIBOX_SOCKET` environment variable (useful for tests and non-standard deployments), falling back to the default `/run/minibox/miniboxd.sock`.

```rust
impl DaemonClient {
    pub fn new() -> Result<Self> {
        let socket_path = std::env::var("MINIBOX_SOCKET")
            .unwrap_or_else(|_| DAEMON_SOCKET_PATH.to_string());
        Ok(Self {
            socket_path: PathBuf::from(socket_path),
        })
    }
}
```

### Error Handling

- Uses `anyhow::Result` + `thiserror` for typed errors
- Wraps all I/O errors with `.context("description")?`
- Follows minibox CLAUDE.md patterns consistently

### Implementation Details

- **No duplication**: Reuses `DaemonRequest`, `DaemonResponse`, `ContainerInfo` types from linuxbox
- **Frame parsing**: Handles newline-delimited JSON framing (already defined in protocol.rs)
- **Base64 decoding**: Handles `ContainerOutput.data` decoding
- **Both sync and async**: Provides async I/O via Tokio; blocking wrapper available for sync callers

### Workspace Integration

Add to `Cargo.toml`:

```toml
members = [
    # ... existing members ...
    "crates/minibox-client",
]
```

### Testing

- **Unit tests**: Mock socket using `tokio::io::DuplexStream`
- **Integration tests**: Real miniboxd connection (Linux+root, gated with feature flag)
- **CI**: Unit tests run on all platforms; integration tests only on Linux in CI

### Dependencies

```toml
[dependencies]
tokio = { workspace = true, features = ["net", "io-util"] }
serde_json = { workspace = true }
linuxbox = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
base64 = { workspace = true }
futures = { workspace = true }
```

---

## Phase 3: mbxctl HTTP Controller

### Objective

Build a standalone HTTP server that wraps minibox-client and exposes REST API for job orchestration. The server is simple (transparent proxy) and stateless initially.

### HTTP API Specification

#### Create and Run Container

**Request:**

```
POST /api/v1/jobs
Content-Type: application/json

{
  "image": "alpine",
  "tag": "latest",
  "command": ["npm", "build"],
  "memory_limit_bytes": 536870912,
  "cpu_weight": 500,
  "stream_output": true,
  "timeout_seconds": 300
}
```

**Response (201 Created):**

```json
{
  "job_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "created"
}
```

#### Get Job Status

**Request:**

```
GET /api/v1/jobs/{job_id}
```

**Response (200 OK):**

```json
{
  "job_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "running",
  "exit_code": null,
  "created_at": "2026-03-24T10:00:00Z",
  "completed_at": null
}
```

Status values: `"created"`, `"running"`, `"completed"`, `"failed"`.

#### Stream Job Output

**Request:**

```
GET /api/v1/jobs/{job_id}/logs
Accept: text/event-stream
```

**Response (200 OK, Content-Type: text/event-stream):**

```
data: {"stream":"stdout","data":"SGVsbG8gV29ybGQ=","timestamp":"2026-03-24T10:00:01Z"}
data: {"stream":"stdout","data":"Zm9v","timestamp":"2026-03-24T10:00:02Z"}
data: {"stream":"stderr","data":"ZXJyb3I=","timestamp":"2026-03-24T10:00:03Z"}
data: {"type":"completed","exit_code":0,"timestamp":"2026-03-24T10:00:04Z"}
```

Each line is a Server-Sent Event (SSE). The `data` field is base64-encoded bytes from the container's output.

#### Stop/Kill Container

**Request:**

```
DELETE /api/v1/jobs/{job_id}
```

**Response (204 No Content).**

### Internal Architecture

**src/server.rs** — HTTP router and middleware

```rust
pub struct Controller {
    client: Arc<DaemonClient>,
    job_tracker: Arc<JobTracker>,
}

impl Controller {
    pub async fn run(&self, addr: &str) -> Result<()> {
        // Setup axum router with handlers
        // Listen on addr
    }
}
```

**src/adapters/jobs.rs** — Job management

```rust
pub struct JobAdapter {
    client: Arc<DaemonClient>,
}

impl JobAdapter {
    pub async fn create_and_run(
        &self,
        req: CreateJobRequest,
    ) -> Result<JobId>;

    pub async fn get_status(
        &self,
        job_id: &JobId,
    ) -> Result<JobStatus>;

    pub async fn stream_logs(
        &self,
        job_id: &JobId,
    ) -> Result<impl Stream<Item = Result<LogEntry>>>;
}
```

**src/models.rs** — Data types

```rust
pub struct JobStatus {
    pub job_id: String,
    pub status: String,  // "created", "running", "completed", "failed"
    pub exit_code: Option<i32>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

pub struct LogEntry {
    pub stream: String,  // "stdout" or "stderr"
    pub data: String,    // Base64-encoded bytes
    pub timestamp: String,
}
```

### State Tracking

**JobTracker** (in-memory, minimal):

- Tracks active job IDs and their status
- Maps job_id → response receiver channel
- No persistence (jobs lost on restart, acceptable for transient dagu runs)
- Cleaned up when job completes

### Error Handling

```rust
pub enum ControllerError {
    #[error("daemon unavailable: {0}")]
    DaemonUnavailable(String),

    #[error("job not found: {job_id}")]
    JobNotFound { job_id: String },

    #[error("container exited with code {code}: {message}")]
    ContainerFailed { code: i32, message: String },

    #[error("timeout waiting for container: {0}")]
    Timeout(String),
}
```

All errors are returned as JSON with HTTP status codes:

- 400 Bad Request: Invalid input
- 404 Not Found: Job doesn't exist
- 500 Internal Server Error: Daemon unavailable or system error

### Installation

**install.sh** (in mbxctl repo):

```bash
#!/usr/bin/env bash

VERSION="v0.1.0"
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Download binary
curl -L "https://releases.example.com/mbxctl/$VERSION/mbxctl-$OS-$ARCH" \
  -o /tmp/mbxctl

chmod +x /tmp/mbxctl

# Install as systemd service (Linux) or standalone (macOS)
if [[ -d /etc/systemd/system && $EUID -eq 0 ]]; then
  mv /tmp/mbxctl /usr/local/bin/
  cat > /etc/systemd/system/mbxctl.service <<'EOF'
[Unit]
Description=Minibox Controller
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/mbxctl --listen localhost:9999
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable mbxctl
  systemctl start mbxctl
else
  mkdir -p ~/.local/bin
  mv /tmp/mbxctl ~/.local/bin/
  echo "Installed to ~/.local/bin/mbxctl"
  echo "Start manually: ~/.local/bin/mbxctl --listen localhost:9999"
fi
```

**Usage:**

```bash
# Standalone
./mbxctl --listen localhost:9999 --socket /run/minibox/miniboxd.sock

# Via curl|install
curl https://releases.example.com/mbxctl/install.sh | bash
```

### Testing

- **Unit tests**: Mock minibox-client, test HTTP handlers
- **Integration tests**: Real minibox-client + mock socket
- **End-to-end**: Real miniboxd (Linux+root required)

---

## Phase 4: mbx-dagu Go Plugin

### Objective

Implement dagu's `Executor` interface as a Go plugin. DAG steps with `executor: minibox` are dispatched to this plugin, which translates them to HTTP calls against mbxctl.

### Dagu Executor Interface

Dagu loads executors as external binaries. The executor reads a JSON request from stdin and writes a JSON response to stdout.

**Input (stdin):**

```json
{
  "command": "alpine",
  "args": ["npm", "build"],
  "env": {"NODE_ENV": "production"},
  "cwd": "/workspace",
  "timeout": 300
}
```

**Output (stdout):**

```json
{
  "exit_code": 0,
  "stdout": "...",
  "stderr": ""
}
```

### Implementation

**cmd/plugin/main.go** — Entry point

```go
package main

import (
    "encoding/json"
    "os"
    "context"
)

func main() {
    var req ExecutionRequest
    json.NewDecoder(os.Stdin).Decode(&req)

    executor := NewMiniboxExecutor(
        os.Getenv("MINIBOX_CONTROLLER"),
    )

    result, err := executor.Execute(context.Background(), &req)
    if err != nil {
        result = &ExecutionResult{ExitCode: 1, Stderr: err.Error()}
    }

    json.NewEncoder(os.Stdout).Encode(result)
}
```

**executor.go** — Minibox executor

```go
type MiniboxExecutor struct {
    ControllerURL string
    HTTPClient    *http.Client
}

func (e *MiniboxExecutor) Execute(ctx context.Context, req *ExecutionRequest) (*ExecutionResult, error) {
    // Parse command[0] as image, remaining args as container command
    image, tag := parseImageRef(req.Command[0])

    jobReq := &CreateJobRequest{
        Image:     image,
        Tag:       tag,
        Command:   req.Command[1:],
        Env:       parseEnv(req.Env),
        StreamOutput: true,
        TimeoutSeconds: req.Timeout,
    }

    // Create job
    jobResp, err := e.createJob(ctx, jobReq)
    if err != nil {
        return nil, err
    }

    // Wait for completion and collect output
    logs, exitCode, err := e.waitForCompletion(ctx, jobResp.JobID)
    if err != nil {
        return nil, err
    }

    return &ExecutionResult{
        ExitCode: exitCode,
        Stdout:   collectStream(logs, "stdout"),
        Stderr:   collectStream(logs, "stderr"),
    }, nil
}
```

### DAG YAML Usage

**Example workflow** (dagu-workflow.yaml):

```yaml
steps:
  - name: build
    executor: minibox
    command: alpine:latest
    args:
      - npm
      - build
    env:
      - NODE_ENV=production

  - name: test
    executor: minibox
    command: alpine:latest
    args:
      - npm
      - test
    depends:
      - build

  - name: deploy
    executor: minibox
    command: ubuntu:22.04
    args:
      - /bin/bash
      - -c
      - |
        apt-get update && apt-get install -y rsync
        rsync -avz /app/dist/ user@host:/var/www/
    depends:
      - test
    env:
      - DEPLOY_KEY=/etc/deploy/key
```

### Registration with Dagu

Dagu discovers executors via:

1. **Environment variable:** `DAGU_EXECUTOR_MINIBOX=/usr/local/bin/mbx-dagu-plugin`
2. **Config file:** `/etc/dagu/config.yaml`

**dagu-config.yaml**:

```yaml
executors:
  minibox:
    binary: /usr/local/bin/mbx-dagu-plugin
    env:
      MINIBOX_CONTROLLER: http://localhost:9999
```

Dagu will invoke the plugin for any step with `executor: minibox`.

### Error Handling

Plugin handles:
- Controller unavailable → return error exit code
- Job timeout → kill job, return timeout error
- Invalid image → propagate daemon error
- Output capture failures → best-effort logging

---

## Integration & Testing

### Test Coverage Matrix

|  | macOS (colima) | Linux (native) |
|---|---|---|
| **Phase 1** (image build) | ✓ Unit | ✓ Integration |
| **Phase 2** (client lib) | ✓ Unit | ✓ Integration |
| **Phase 3** (mbxctl) | ✓ Unit | ✓ Integration |
| **Phase 4** (mbx-dagu) | ✓ Unit | ✓ E2E |

### Unit Tests (All Platforms)

- minibox-client: Mock socket, test frame parsing/decoding
- mbxctl: Mock client, test HTTP handlers and status transitions
- mbx-dagu: Mock HTTP server, test executor interface

### Integration Tests (Linux + Root)

- minibox-client + real miniboxd
- mbxctl + real miniboxd + HTTP client
- Full stack: dagu container + mbxctl + miniboxd

### End-to-End Test Scenario

```bash
# Setup
miniboxd &
mbxctl --listen localhost:9999 &

# Run dagu with a test workflow
minibox run dagu:minibox \
  -e MINIBOX_CONTROLLER=http://localhost:9999 \
  -v /run/minibox/miniboxd.sock:/run/minibox/miniboxd.sock:ro \
  -- /dagu dag run example-workflow.yaml

# Assertions
# - All steps complete successfully
# - Output is captured and visible
# - Exit codes are correct
# - Dependencies are respected
```

---

## Deployment Models

### Local Development

```bash
# Terminal 1: minibox daemon
sudo ./target/release/miniboxd

# Terminal 2: controller
./mbxctl --listen localhost:9999

# Terminal 3: dagu in minibox
minibox run dagu:minibox \
  -e MINIBOX_CONTROLLER=http://localhost:9999 \
  -v /run/minibox/miniboxd.sock:/run/minibox/miniboxd.sock:ro \
  -- /dagu server

# Web UI at http://localhost:8080
```

### Production (Linux Server)

```bash
# Install as systemd services
curl https://releases.example.com/mbxctl/install.sh | sudo bash

# miniboxd is already installed, ensure it's running
sudo systemctl start miniboxd

# Start controller
sudo systemctl start mbxctl

# Run dagu container (can be long-lived or managed via systemd/k8s)
minibox run dagu:minibox \
  -e MINIBOX_CONTROLLER=http://localhost:9999 \
  -v /run/minibox/miniboxd.sock:/run/minibox/miniboxd.sock:ro \
  -- /dagu server

# Access Web UI via reverse proxy (nginx, etc.)
```

---

## Design Decisions & Rationales

| Decision | Rationale | Alternative Considered |
|----------|-----------|------------------------|
| **Extract minibox-client library** | Reusable component, avoids duplication, follows DRY principle | Embed in each consumer |
| **HTTP for controller API** | Language-agnostic, easy debugging (curl), standard (SSE for streaming) | gRPC (more complex, less debuggable for simple cases) |
| **Transparent proxy (stateless)** | Simple, fast, dagu owns job history | Persistent store (adds complexity, potential SPOF) |
| **Server-Sent Events (SSE)** | Standard for unidirectional streams, works in browsers | WebSocket (overkill), polling (inefficient) |
| **Unix socket from container** | Reuses existing minibox protocol, secure, doesn't require special networking | TCP loopback (adds network setup overhead) |
| **Go for dagu plugin** | Aligns with dagu's ecosystem, dagu maintainers familiar | Rust plugin (adds build complexity to minibox workspace) |
| **Four separate phases** | Each phase is independently valuable and testable | Monolithic implementation (harder to debug) |

---

## Success Criteria

### Phase 1: Containerized Dagu
- ✅ Image builds without errors
- ✅ Runs in minibox (`minibox run dagu:minibox`)
- ✅ Web UI accessible at port 8080
- ✅ Published to ghcr.io (or registry)

### Phase 2: minibox-client Library
- ✅ Unit tests pass (mock socket)
- ✅ Integration tests pass on Linux (real miniboxd)
- ✅ API is clean, extensible, reusable
- ✅ Error handling consistent with minibox patterns

### Phase 3: mbxctl Controller
- ✅ HTTP API works against real miniboxd
- ✅ Streaming works (SSE events received correctly)
- ✅ Error handling is robust and informative
- ✅ Can be installed via curl|sh
- ✅ Works on both macOS and Linux

### Phase 4: mbx-dagu Plugin
- ✅ DAG with `executor: minibox` steps runs end-to-end
- ✅ Output captured and visible in dagu Web UI
- ✅ Exit codes propagate correctly
- ✅ Complex workflows (dependencies) work
- ✅ Resource limits (memory, CPU) are applied

### All Phases
- ✅ Works on macOS (via colima) and Linux
- ✅ Security: socket is read-only mount, no privilege escalation
- ✅ Performance: overhead negligible vs direct `minibox run`
- ✅ Documentation: clear setup instructions for all platforms

---

## Out of Scope

- Custom resource definitions (CRDs) / Kubernetes integration
- Job persistence across restarts
- Advanced scheduling (priority, backoff, retries)
- Multi-tenant isolation (single-user assumption)
- HTTPS for mbxctl (assumes localhost use)

These can be addressed in future iterations.

---

## References

- [Dagu Documentation](https://dagu-org.github.io/)
- [Minibox Architecture](../../CLAUDE.md)
- [Minibox Protocol](../../../crates/linuxbox/src/protocol.rs)
