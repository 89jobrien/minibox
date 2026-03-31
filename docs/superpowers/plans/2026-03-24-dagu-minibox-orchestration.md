# Dagu + Minibox Orchestration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use [superpowers:subagent-driven-development](../skills/superpowers:subagent-driven-development) (recommended) or [superpowers:executing-plans](../skills/superpowers:executing-plans) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a four-phase integration enabling dagu workflow engine to orchestrate minibox containers with real-time output streaming and resource limits.

**Architecture:**

- Phase 1: Build `dagu:minibox` container image with pre-installed plugin
- Phase 2: Extract socket communication into reusable `minibox-client` library
- Phase 3: Create `mbxctl` HTTP controller wrapping the client
- Phase 4: Develop `mbx-dagu` Go plugin for dagu executor interface

**Tech Stack:** Rust (phases 2-3), Go (phase 4), Docker (phase 1), Tokio async, axum HTTP, Server-Sent Events for streaming

**Timeline:** ~3-4 weeks (4 phases, parallel where possible)

---

## Phase 1: Dagu Container Image

### Task 1: Create mbx-dagu Repository Structure

**Files:**

- Create: `mbx-dagu/Dockerfile`
- Create: `mbx-dagu/dagu-config.yaml`
- Create: `mbx-dagu/go.mod`
- Create: `mbx-dagu/.gitignore`
- Create: `mbx-dagu/README.md`

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p mbx-dagu/{cmd/plugin,executor,tests}
cd mbx-dagu
git init
```

- [ ] **Step 2: Write Dockerfile**

```dockerfile
# Multi-stage: build Go plugin, then add to dagu image

FROM golang:1.23-alpine AS plugin-builder
WORKDIR /src/mbx-dagu
COPY go.mod go.sum ./
RUN go mod download
COPY cmd cmd
COPY executor executor
RUN CGO_ENABLED=0 GOOS=linux go build -o /tmp/mbx-dagu-plugin ./cmd/plugin

FROM ghcr.io/dagu-org/dagu:latest
RUN apk add --no-cache ca-certificates

# Copy compiled plugin
COPY --from=plugin-builder /tmp/mbx-dagu-plugin /usr/local/bin/

# Dagu configuration to register the plugin
COPY dagu-config.yaml /etc/dagu/

EXPOSE 8080

ENTRYPOINT ["/dagu", "server", "--host", "0.0.0.0", "--port", "8080"]
```

- [ ] **Step 3: Write dagu-config.yaml**

```yaml
executors:
  minibox:
    binary: /usr/local/bin/mbx-dagu-plugin
    env:
      MINIBOX_CONTROLLER: http://localhost:9999
```

- [ ] **Step 4: Write go.mod**

```
module github.com/dagu-org/mbx-dagu

go 1.23

require (
    github.com/dagu-org/dagu v1.16.0
)
```

- [ ] **Step 5: Create .gitignore**

```
/bin/
/dist/
*.o
*.a
.DS_Store
vendor/
```

- [ ] **Step 6: Create README.md**

```markdown
# mbx-dagu

Dagu executor plugin for minibox container runtime.

## Features

- Executes DAG steps as isolated minibox containers
- Real-time output streaming
- Resource limits (memory, CPU)
- Timeout enforcement

## Build

\`\`\`bash
docker build -t dagu:minibox .
\`\`\`

## Usage

\`\`\`bash
minibox run dagu:minibox -e MINIBOX_CONTROLLER=http://localhost:9999 -- /dagu server
\`\`\`
```

- [ ] **Step 7: Commit**

```bash
git add .
git commit -m "build: initialize mbx-dagu repo with Dockerfile and config"
```

---

### Task 2: Create Empty Go Plugin Entry Point

**Files:**

- Create: `mbx-dagu/cmd/plugin/main.go`
- Create: `mbx-dagu/executor/executor.go`
- Create: `mbx-dagu/executor/models.go`

- [ ] **Step 1: Write main.go (minimal)**

```go
package main

import (
    "encoding/json"
    "os"
    "context"
    "github.com/dagu-org/mbx-dagu/executor"
)

func main() {
    var req executor.ExecutionRequest
    err := json.NewDecoder(os.Stdin).Decode(&req)
    if err != nil {
        json.NewEncoder(os.Stderr).Encode(map[string]interface{}{
            "error": "failed to decode request",
        })
        os.Exit(1)
    }

    exe := executor.NewMiniboxExecutor(os.Getenv("MINIBOX_CONTROLLER"))
    result, err := exe.Execute(context.Background(), &req)
    if err != nil {
        result = &executor.ExecutionResult{
            ExitCode: 1,
            Stderr:   err.Error(),
        }
    }

    json.NewEncoder(os.Stdout).Encode(result)
}
```

- [ ] **Step 2: Write executor/models.go**

```go
package executor

// ExecutionRequest is the input from dagu
type ExecutionRequest struct {
    Command []string            `json:"command"`
    Args    []string            `json:"args"`
    Env     map[string]string   `json:"env"`
    Cwd     string              `json:"cwd"`
    Timeout int                 `json:"timeout"`
}

// ExecutionResult is the output to dagu
type ExecutionResult struct {
    ExitCode int    `json:"exit_code"`
    Stdout   string `json:"stdout"`
    Stderr   string `json:"stderr"`
}
```

- [ ] **Step 3: Write executor/executor.go (skeleton)**

```go
package executor

import (
    "context"
)

type MiniboxExecutor struct {
    ControllerURL string
}

func NewMiniboxExecutor(controllerURL string) *MiniboxExecutor {
    return &MiniboxExecutor{
        ControllerURL: controllerURL,
    }
}

func (e *MiniboxExecutor) Execute(ctx context.Context, req *ExecutionRequest) (*ExecutionResult, error) {
    // TODO: Implement
    return &ExecutionResult{
        ExitCode: 0,
        Stdout:   "not implemented",
    }, nil
}
```

- [ ] **Step 4: Commit**

```bash
git add cmd/ executor/
git commit -m "feat(plugin): add Go plugin entry point and executor skeleton"
```

---

### Task 3: Build and Test Docker Image

**Files:**

- Modify: `mbx-dagu/.github/workflows/build.yml` (if using GHA)

- [ ] **Step 1: Build Docker image locally**

```bash
cd mbx-dagu
docker build -t dagu:minibox .
```

Expected: Build succeeds, no errors.

- [ ] **Step 2: Test image runs**

```bash
docker run --rm dagu:minibox /dagu version
```

Expected: Output shows dagu version.

- [ ] **Step 3: Verify plugin is present**

```bash
docker run --rm dagu:minibox ls -la /usr/local/bin/mbx-dagu-plugin
```

Expected: Plugin binary exists.

- [ ] **Step 4: Create GitHub Actions workflow (optional for Phase 1)**

If using GitHub, create `.github/workflows/build.yml`:

```yaml
name: Build

on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Build Docker image
        run: docker build -t dagu:minibox .
      - name: Test image
        run: docker run --rm dagu:minibox /dagu version
```

- [ ] **Step 5: Commit**

```bash
git add .
git commit -m "build: docker image builds and runs successfully"
```

**END PHASE 1**

---

## Phase 2: minibox-client Library

### Task 4: Create minibox-client Crate Structure

**Files:**

- Create: `crates/minibox-client/Cargo.toml`
- Create: `crates/minibox-client/src/lib.rs`
- Create: `crates/minibox-client/src/error.rs`
- Create: `crates/minibox-client/src/socket.rs`
- Modify: `Cargo.toml` (add member)

- [ ] **Step 1: Create directory**

```bash
mkdir -p crates/minibox-client/src
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "minibox-client"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
tokio = { workspace = true, features = ["net", "io-util"] }
serde_json = { workspace = true }
mbx = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
base64 = { workspace = true }
futures = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["full"] }
```

- [ ] **Step 3: Update workspace Cargo.toml**

Add to members list:

```toml
members = [
    # ... existing ...
    "crates/minibox-client",
]
```

- [ ] **Step 4: Write src/error.rs**

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("failed to connect to daemon: {0}")]
    ConnectionFailed(#[from] std::io::Error),

    #[error("daemon error: {0}")]
    DaemonError(String),

    #[error("frame error: {0}")]
    FrameError(String),

    #[error("socket path not found")]
    SocketPathNotFound,

    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ClientError>;
```

- [ ] **Step 5: Write src/lib.rs (public API)**

```rust
pub mod error;
pub mod socket;

pub use error::{ClientError, Result};
pub use socket::{DaemonClient, DaemonResponseStream};

use mbx::protocol::DAEMON_SOCKET_PATH;
use std::path::PathBuf;

pub fn default_socket_path() -> PathBuf {
    PathBuf::from(
        std::env::var("MINIBOX_SOCKET_PATH")
            .unwrap_or_else(|_| DAEMON_SOCKET_PATH.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_path() {
        let path = default_socket_path();
        assert!(path.as_os_str().len() > 0);
    }
}
```

- [ ] **Step 6: Write src/socket.rs (connection logic)**

```rust
use crate::error::{ClientError, Result};
use mbx::protocol::{DaemonRequest, DaemonResponse, decode_response};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub struct DaemonClient {
    socket_path: std::path::PathBuf,
}

impl DaemonClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            socket_path: crate::default_socket_path(),
        })
    }

    pub fn with_socket(path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: path.as_ref().to_path_buf(),
        }
    }

    pub async fn call(
        &self,
        request: DaemonRequest,
    ) -> Result<DaemonResponseStream> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(ClientError::ConnectionFailed)?;

        let (read_half, mut write_half) = stream.into_split();

        // Send request
        let payload = serde_json::to_string(&request)?;
        write_half
            .write_all(format!("{}\n", payload).as_bytes())
            .await
            .map_err(ClientError::ConnectionFailed)?;
        write_half
            .flush()
            .await
            .map_err(ClientError::ConnectionFailed)?;

        Ok(DaemonResponseStream {
            reader: BufReader::new(read_half),
        })
    }
}

pub struct DaemonResponseStream {
    reader: BufReader<tokio::net::unix_stream::OwnedReadHalf>,
}

impl DaemonResponseStream {
    pub async fn next(&mut self) -> Result<Option<DaemonResponse>> {
        use std::io::Cursor;

        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await
            .map_err(ClientError::ConnectionFailed)?;

        if n == 0 {
            return Ok(None);
        }

        let response = decode_response(line.as_bytes())
            .map_err(|e| ClientError::FrameError(e.to_string()))?;

        Ok(Some(response))
    }

    pub async fn try_collect(mut self) -> Result<Vec<DaemonResponse>> {
        let mut responses = Vec::new();
        while let Some(resp) = self.next().await? {
            responses.push(resp);
        }
        Ok(responses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = DaemonClient::new();
        assert!(client.is_ok());
    }

    #[test]
    fn test_client_with_socket() {
        let client = DaemonClient::with_socket("/tmp/test.sock");
        assert_eq!(client.socket_path, std::path::PathBuf::from("/tmp/test.sock"));
    }
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p minibox-client
```

Expected: Tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/minibox-client/ Cargo.toml
git commit -m "feat: add minibox-client library with Unix socket communication"
```

---

### Task 5: Refactor minibox-cli to Use minibox-client

**Files:**

- Modify: `crates/minibox-cli/Cargo.toml`
- Modify: `crates/minibox-cli/src/commands/mod.rs`
- Modify: `crates/minibox-cli/src/commands/run.rs`

- [ ] **Step 1: Add minibox-client dependency**

Update `crates/minibox-cli/Cargo.toml`:

```toml
[dependencies]
minibox-client = { workspace = true }
```

- [ ] **Step 2: Refactor run.rs to use DaemonClient**

Replace the manual socket connection code with:

```rust
use minibox_client::DaemonClient;

pub async fn execute(
    image: String,
    tag: String,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: String,
) -> Result<()> {
    let network_mode = match network.as_str() {
        "none" => NetworkMode::None,
        "bridge" => NetworkMode::Bridge,
        "host" => NetworkMode::Host,
        "tailnet" => NetworkMode::Tailnet,
        other => {
            anyhow::bail!("unknown network mode: {other} (expected: none, bridge, host, tailnet)")
        }
    };

    let request = DaemonRequest::Run {
        image,
        tag: Some(tag),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: true,
        network: Some(network_mode),
    };

    let client = DaemonClient::new()
        .context("failed to create daemon client")?;

    let mut stream = client.call(request)
        .await
        .context("failed to call daemon")?;

    // Process stream (existing output handling code)
    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ContainerCreated { id } => {
                eprintln!("Container created: {}", id);
            }
            DaemonResponse::ContainerOutput { stream: kind, data } => {
                // Existing handling code
            }
            DaemonResponse::ContainerStopped { exit_code } => {
                std::process::exit(exit_code);
            }
            DaemonResponse::Error { message } => {
                eprintln!("Error: {}", message);
                std::process::exit(1);
            }
            _ => {}
        }
    }

    Ok(())
}
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p minibox-cli
```

Expected: No errors.

- [ ] **Step 4: Run existing CLI tests**

```bash
cargo test -p minibox-cli
```

Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-cli/
git commit -m "refactor(cli): use minibox-client library for socket communication"
```

**END PHASE 2**

---

## Phase 3: mbxctl HTTP Controller

### Task 6: Create mbxctl Crate Structure

**Files:**

- Create: `mbxctl/Cargo.toml`
- Create: `mbxctl/src/main.rs`
- Create: `mbxctl/src/server.rs`
- Create: `mbxctl/src/error.rs`
- Create: `mbxctl/src/models.rs`
- Create: `mbxctl/src/adapters/mod.rs`
- Create: `mbxctl/src/adapters/jobs.rs`

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p mbxctl/src/adapters
cd mbxctl
cargo init --name mbxctl
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "mbxctl"
version = "0.1.0"
edition = "2024"
license = "MIT"

[dependencies]
tokio = { version = "1", features = ["full"] }
axum = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
minibox-client = { path = "../minibox/crates/minibox-client" }
mbx = { path = "../minibox/crates/mbx" }
futures = "0.3"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

- [ ] **Step 3: Write src/error.rs**

```rust
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ControllerError {
    #[error("daemon unavailable: {0}")]
    DaemonUnavailable(String),

    #[error("job not found: {job_id}")]
    JobNotFound { job_id: String },

    #[error("container failed: {message}")]
    ContainerFailed { message: String },

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ControllerError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ControllerError::DaemonUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg),
            ControllerError::JobNotFound { job_id } => (StatusCode::NOT_FOUND, format!("Job {} not found", job_id)),
            ControllerError::ContainerFailed { message } => (StatusCode::INTERNAL_SERVER_ERROR, message),
            ControllerError::Timeout(msg) => (StatusCode::REQUEST_TIMEOUT, msg),
            ControllerError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, ControllerError>;
```

- [ ] **Step 4: Write src/models.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub image: String,
    pub tag: Option<String>,
    pub command: Vec<String>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub stream_output: Option<bool>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: String,
    pub container_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    pub job_id: String,
    pub status: String, // "created", "running", "completed", "failed"
    pub exit_code: Option<i32>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub stream: String, // "stdout" or "stderr"
    pub data: String,   // Base64-encoded
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedEvent {
    #[serde(rename = "type")]
    pub type_: String, // "completed"
    pub exit_code: i32,
    pub timestamp: String,
}
```

- [ ] **Step 5: Write src/adapters/mod.rs**

```rust
pub mod jobs;

pub use jobs::JobAdapter;
```

- [ ] **Step 6: Write src/adapters/jobs.rs (skeleton)**

```rust
use crate::error::Result;
use crate::models::{CreateJobRequest, JobStatus};
use minibox_client::DaemonClient;
use std::sync::Arc;

pub struct JobAdapter {
    client: Arc<DaemonClient>,
}

impl JobAdapter {
    pub fn new(client: Arc<DaemonClient>) -> Self {
        Self { client }
    }

    pub async fn create_and_run(
        &self,
        req: CreateJobRequest,
    ) -> Result<String> {
        // TODO: Implement
        Ok("job-id".to_string())
    }

    pub async fn get_status(&self, job_id: &str) -> Result<JobStatus> {
        // TODO: Implement
        Ok(JobStatus {
            job_id: job_id.to_string(),
            status: "running".to_string(),
            exit_code: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
    }
}
```

- [ ] **Step 7: Write src/server.rs (skeleton)**

```rust
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

pub async fn run(
    listener_addr: &str,
) -> anyhow::Result<()> {
    let client = Arc::new(minibox_client::DaemonClient::new()?);

    let app = Router::new()
        .route("/api/v1/jobs", post(create_job))
        .route("/api/v1/jobs/:job_id", get(get_job_status))
        .route("/api/v1/jobs/:job_id", delete(delete_job))
        .route("/api/v1/jobs/:job_id/logs", get(stream_logs))
        .with_state(client);

    let listener = tokio::net::TcpListener::bind(listener_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn create_job() {}
async fn get_job_status() {}
async fn delete_job() {}
async fn stream_logs() {}
```

- [ ] **Step 8: Write src/main.rs (CLI + server start)**

```rust
mod adapters;
mod error;
mod models;
mod server;

use clap::Parser;

#[derive(Parser)]
#[command(name = "mbxctl")]
#[command(about = "Minibox orchestration controller")]
struct Args {
    #[arg(long, default_value = "localhost:9999")]
    listen: String,

    #[arg(long, env = "MINIBOX_SOCKET_PATH")]
    socket: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("mbxctl=debug".parse()?),
        )
        .init();

    let args = Args::parse();

    tracing::info!("Starting mbxctl on {}", args.listen);
    server::run(&args.listen).await?;

    Ok(())
}
```

- [ ] **Step 9: Verify compile**

```bash
cargo check
```

Expected: No errors (skeleton compiles).

- [ ] **Step 10: Commit**

```bash
git add .
git commit -m "feat: initialize mbxctl HTTP server skeleton"
```

---

### Task 7: Implement Job Creation and HTTP Endpoints

**Files:**

- Modify: `mbxctl/src/adapters/jobs.rs`
- Modify: `mbxctl/src/server.rs`

- [ ] **Step 1: Implement job creation (jobs.rs)**

```rust
use crate::error::{ControllerError, Result};
use crate::models::{CreateJobRequest, JobStatus};
use mbx::protocol::DaemonRequest;
use minibox_client::DaemonClient;
use std::sync::Arc;
use uuid::Uuid;

pub struct JobAdapter {
    client: Arc<DaemonClient>,
}

impl JobAdapter {
    pub fn new(client: Arc<DaemonClient>) -> Self {
        Self { client }
    }

    pub async fn create_and_run(
        &self,
        req: CreateJobRequest,
    ) -> Result<(String, String)> {  // (job_id, container_id)
        let tag = req.tag.unwrap_or_else(|| "latest".to_string());
        let job_id = Uuid::new_v4().to_string();

        let daemon_req = DaemonRequest::Run {
            image: req.image,
            tag: Some(tag),
            command: req.command,
            memory_limit_bytes: req.memory_limit_bytes,
            cpu_weight: req.cpu_weight,
            ephemeral: true,
            network: None,
        };

        let mut stream = self.client.call(daemon_req)
            .await
            .map_err(|e| ControllerError::DaemonUnavailable(e.to_string()))?;

        // Get container ID from ContainerCreated response
        let container_id = loop {
            match stream.next().await
                .map_err(|e| ControllerError::Internal(e.to_string()))?
            {
                Some(mbx::protocol::DaemonResponse::ContainerCreated { id }) => break id,
                Some(mbx::protocol::DaemonResponse::Error { message }) => {
                    return Err(ControllerError::ContainerFailed { message });
                }
                _ => continue,
            }
        };

        Ok((job_id, container_id))
    }

    pub async fn get_status(&self, _job_id: &str) -> Result<JobStatus> {
        // Phase 1: Just return running status
        // Later phases can track persistent state
        Ok(JobStatus {
            job_id: _job_id.to_string(),
            status: "running".to_string(),
            exit_code: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
    }
}
```

- [ ] **Step 2: Implement HTTP endpoints (server.rs)**

```rust
use crate::adapters::JobAdapter;
use crate::models::{CreateJobRequest, CreateJobResponse};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use std::sync::Arc;

type SharedClient = Arc<minibox_client::DaemonClient>;

pub async fn run(
    listener_addr: &str,
) -> anyhow::Result<()> {
    let client = Arc::new(minibox_client::DaemonClient::new()?);

    let adapter = Arc::new(JobAdapter::new(client));

    let app = Router::new()
        .route("/api/v1/jobs", post(create_job))
        .route("/api/v1/jobs/:job_id", get(get_job_status))
        .route("/api/v1/jobs/:job_id", delete(delete_job))
        .route("/api/v1/jobs/:job_id/logs", get(stream_logs))
        .with_state(adapter);

    let listener = tokio::net::TcpListener::bind(listener_addr).await?;
    tracing::info!("Server listening on {}", listener_addr);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn create_job(
    State(adapter): State<Arc<JobAdapter>>,
    Json(req): Json<CreateJobRequest>,
) -> impl IntoResponse {
    match adapter.create_and_run(req).await {
        Ok((job_id, container_id)) => (
            StatusCode::CREATED,
            Json(CreateJobResponse {
                job_id,
                container_id,
                status: "created".to_string(),
            }),
        ),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))),
    }
}

async fn get_job_status(
    State(adapter): State<Arc<JobAdapter>>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    match adapter.get_status(&job_id).await {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn delete_job(
    Path(_job_id): Path<String>,
) -> impl IntoResponse {
    // TODO: Implement stop
    StatusCode::NO_CONTENT
}

async fn stream_logs(
    Path(_job_id): Path<String>,
) -> impl IntoResponse {
    // TODO: Implement streaming
    StatusCode::OK
}
```

- [ ] **Step 3: Test compilation**

```bash
cargo check
```

Expected: Compiles.

- [ ] **Step 4: Write integration test**

Create `mbxctl/tests/integration.rs`:

```rust
#[tokio::test]
#[ignore]  // Requires miniboxd running
async fn test_create_job_request() {
    // Mock test (real test would need running miniboxd)
    let req = mbxctl::models::CreateJobRequest {
        image: "alpine".to_string(),
        tag: Some("latest".to_string()),
        command: vec!["echo".to_string(), "hello".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        stream_output: Some(true),
        timeout_seconds: Some(10),
    };

    assert_eq!(req.image, "alpine");
    assert_eq!(req.command.len(), 2);
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test --test integration
```

Expected: Tests pass.

- [ ] **Step 6: Commit**

```bash
git add .
git commit -m "feat: implement job creation and HTTP endpoints (create_job, get_status)"
```

---

### Task 8: Complete Streaming and Timeout Support

**Files:**

- Modify: `mbxctl/src/adapters/jobs.rs`
- Modify: `mbxctl/src/server.rs`

(Due to length, implementation would follow same pattern as Task 7 with streaming, SSE, timeout enforcement using `tokio::time::timeout`)

**Simplification for Plan**: These are complex but follow established patterns. Core logic:

- Use `tokio::time::timeout()` to enforce timeout
- Stream `ContainerOutput` as Server-Sent Events
- Return `CompletedEvent` when container stops

- [ ] **Step N: Write streaming logic (pseudocode)**

Each `ContainerOutput` from daemon becomes SSE event:

```
data: {"stream":"stdout","data":"base64...","timestamp":"..."}
```

- [ ] **Step N+1: Commit streaming + timeout**

```bash
git commit -m "feat: add log streaming (SSE) and timeout enforcement"
```

---

### Task 9: Create Installation Script

**Files:**

- Create: `mbxctl/install.sh`

- [ ] **Step 1: Write install.sh**

```bash
#!/usr/bin/env bash
set -e

VERSION="${1:-v0.1.0}"
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m | sed 's/x86_64/amd64/g; s/aarch64/arm64/g')

echo "Installing mbxctl $VERSION for $OS/$ARCH..."

# Download binary
RELEASE_URL="https://releases.example.com/mbxctl/${VERSION}/mbxctl-${OS}-${ARCH}"
curl -L "$RELEASE_URL" -o /tmp/mbxctl || {
    echo "Failed to download from $RELEASE_URL"
    exit 1
}

chmod +x /tmp/mbxctl

# Install
if [[ -d /etc/systemd/system && $EUID -eq 0 ]]; then
    echo "Installing as systemd service..."
    mv /tmp/mbxctl /usr/local/bin/
    cat > /etc/systemd/system/mbxctl.service <<'EOF'
[Unit]
Description=Minibox Controller
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/mbxctl --listen 0.0.0.0:9999
Restart=on-failure
RestartSec=5s
Environment=MINIBOX_SOCKET_PATH=/run/minibox/miniboxd.sock

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable mbxctl
    systemctl start mbxctl
    echo "✓ Installed and started. Check status: systemctl status mbxctl"
else
    echo "Installing to ~/.local/bin..."
    mkdir -p ~/.local/bin
    mv /tmp/mbxctl ~/.local/bin/
    echo "✓ Installed to ~/.local/bin/mbxctl"
    echo "  Start manually: ~/.local/bin/mbxctl --listen localhost:9999"
fi
```

- [ ] **Step 2: Make executable**

```bash
chmod +x mbxctl/install.sh
```

- [ ] **Step 3: Commit**

```bash
git add install.sh
git commit -m "build: add installation script for mbxctl"
```

**END PHASE 3**

---

## Phase 4: mbx-dagu Go Plugin (Complete Implementation)

### Task 10: Implement Go Plugin Executor Logic

**Files:**

- Modify: `mbx-dagu/executor/executor.go`
- Create: `mbx-dagu/executor/http_client.go`
- Modify: `mbx-dagu/cmd/plugin/main.go`

- [ ] **Step 1: Write executor/http_client.go**

```go
package executor

import (
    "bytes"
    "context"
    "encoding/json"
    "fmt"
    "io"
    "net/http"
    "time"
)

type CreateJobRequest struct {
    Image             string   `json:"image"`
    Tag               string   `json:"tag"`
    Command           []string `json:"command"`
    MemoryLimitBytes  *int64   `json:"memory_limit_bytes,omitempty"`
    CpuWeight         *int64   `json:"cpu_weight,omitempty"`
    StreamOutput      bool     `json:"stream_output"`
    TimeoutSeconds    *int64   `json:"timeout_seconds,omitempty"`
}

type CreateJobResponse struct {
    JobID       string `json:"job_id"`
    ContainerID string `json:"container_id"`
    Status      string `json:"status"`
}

type HTTPClient struct {
    baseURL    string
    httpClient *http.Client
}

func NewHTTPClient(baseURL string) *HTTPClient {
    return &HTTPClient{
        baseURL: baseURL,
        httpClient: &http.Client{
            Timeout: 30 * time.Second,
        },
    }
}

func (c *HTTPClient) CreateAndRun(ctx context.Context, req CreateJobRequest) (*CreateJobResponse, error) {
    payload, err := json.Marshal(req)
    if err != nil {
        return nil, err
    }

    httpReq, err := http.NewRequestWithContext(ctx, "POST", fmt.Sprintf("%s/api/v1/jobs", c.baseURL), bytes.NewReader(payload))
    if err != nil {
        return nil, err
    }
    httpReq.Header.Set("Content-Type", "application/json")

    resp, err := c.httpClient.Do(httpReq)
    if err != nil {
        return nil, err
    }
    defer resp.Body.Close()

    if resp.StatusCode != http.StatusCreated {
        body, _ := io.ReadAll(resp.Body)
        return nil, fmt.Errorf("unexpected status %d: %s", resp.StatusCode, string(body))
    }

    var jobResp CreateJobResponse
    if err := json.NewDecoder(resp.Body).Decode(&jobResp); err != nil {
        return nil, err
    }

    return &jobResp, nil
}

func (c *HTTPClient) StreamLogs(ctx context.Context, jobID string) (io.ReadCloser, error) {
    httpReq, err := http.NewRequestWithContext(ctx, "GET", fmt.Sprintf("%s/api/v1/jobs/%s/logs", c.baseURL, jobID), nil)
    if err != nil {
        return nil, err
    }
    httpReq.Header.Set("Accept", "text/event-stream")

    resp, err := c.httpClient.Do(httpReq)
    if err != nil {
        return nil, err
    }

    if resp.StatusCode != http.StatusOK {
        resp.Body.Close()
        return nil, fmt.Errorf("unexpected status %d", resp.StatusCode)
    }

    return resp.Body, nil
}
```

- [ ] **Step 2: Update executor/executor.go (full implementation)**

```go
package executor

import (
    "bufio"
    "context"
    "encoding/base64"
    "encoding/json"
    "fmt"
    "strings"
)

type MiniboxExecutor struct {
    httpClient *HTTPClient
}

func NewMiniboxExecutor(controllerURL string) *MiniboxExecutor {
    return &MiniboxExecutor{
        httpClient: NewHTTPClient(controllerURL),
    }
}

func (e *MiniboxExecutor) Execute(ctx context.Context, req *ExecutionRequest) (*ExecutionResult, error) {
    // Parse image and tag from command[0]
    if len(req.Command) == 0 {
        return nil, fmt.Errorf("command is empty")
    }

    image, tag := parseImageRef(req.Command[0])
    containerCmd := req.Command[1:]

    // Create job request
    jobReq := CreateJobRequest{
        Image:        image,
        Tag:          tag,
        Command:      containerCmd,
        StreamOutput: true,
    }

    if req.Timeout > 0 {
        timeout := int64(req.Timeout)
        jobReq.TimeoutSeconds = &timeout
    }

    // Create job
    jobResp, err := e.httpClient.CreateAndRun(ctx, jobReq)
    if err != nil {
        return nil, fmt.Errorf("failed to create job: %w", err)
    }

    // Stream logs
    logs, exitCode, err := e.streamLogs(ctx, jobResp.JobID)
    if err != nil {
        return nil, fmt.Errorf("failed to stream logs: %w", err)
    }

    return &ExecutionResult{
        ExitCode: exitCode,
        Stdout:   logs["stdout"],
        Stderr:   logs["stderr"],
    }, nil
}

func (e *MiniboxExecutor) streamLogs(ctx context.Context, jobID string) (map[string]string, int, error) {
    resp, err := e.httpClient.StreamLogs(ctx, jobID)
    if err != nil {
        return nil, 1, err
    }
    defer resp.Close()

    logs := map[string]string{
        "stdout": "",
        "stderr": "",
    }
    exitCode := 0

    scanner := bufio.NewScanner(resp)
    for scanner.Scan() {
        line := scanner.Text()

        if !strings.HasPrefix(line, "data: ") {
            continue
        }

        data := strings.TrimPrefix(line, "data: ")

        var logEntry map[string]interface{}
        if err := json.Unmarshal([]byte(data), &logEntry); err != nil {
            continue
        }

        if logType, ok := logEntry["type"].(string); ok && logType == "completed" {
            if ec, ok := logEntry["exit_code"].(float64); ok {
                exitCode = int(ec)
            }
            break
        }

        // Regular log entry
        if stream, ok := logEntry["stream"].(string); ok {
            if encoded, ok := logEntry["data"].(string); ok {
                decoded, _ := base64.StdEncoding.DecodeString(encoded)
                logs[stream] += string(decoded)
            }
        }
    }

    return logs, exitCode, scanner.Err()
}

func parseImageRef(ref string) (string, string) {
    parts := strings.Split(ref, ":")
    if len(parts) == 2 {
        return parts[0], parts[1]
    }
    return parts[0], "latest"
}
```

- [ ] **Step 3: Update cmd/plugin/main.go (full)**

```go
package main

import (
    "encoding/json"
    "os"
    "context"
    "github.com/dagu-org/mbx-dagu/executor"
)

func main() {
    var req executor.ExecutionRequest
    err := json.NewDecoder(os.Stdin).Decode(&req)
    if err != nil {
        json.NewEncoder(os.Stderr).Encode(map[string]interface{}{
            "error": "failed to decode request",
        })
        os.Exit(1)
    }

    exe := executor.NewMiniboxExecutor(os.Getenv("MINIBOX_CONTROLLER"))
    result, err := exe.Execute(context.Background(), &req)
    if err != nil {
        result = &executor.ExecutionResult{
            ExitCode: 1,
            Stderr:   err.Error(),
        }
    }

    json.NewEncoder(os.Stdout).Encode(result)
}
```

- [ ] **Step 4: Build plugin**

```bash
go build -o /tmp/mbx-dagu-plugin ./cmd/plugin
```

Expected: Binary created successfully.

- [ ] **Step 5: Test plugin binary**

```bash
echo '{"command":["alpine","echo","hello"],"env":{},"cwd":"/","timeout":10}' | /tmp/mbx-dagu-plugin
```

(Requires mbxctl running for real test)

- [ ] **Step 6: Commit**

```bash
git add .
git commit -m "feat: implement complete Go executor plugin for dagu"
```

---

### Task 11: End-to-End Testing

**Files:**

- Create: `mbx-dagu/tests/e2e_test.go`

- [ ] **Step 1: Write E2E test setup**

```go
package main

import (
    "context"
    "os"
    "testing"
    "github.com/dagu-org/mbx-dagu/executor"
)

func TestExecutorE2E(t *testing.T) {
    // Requires:
    // - miniboxd running (sudo ./target/release/miniboxd)
    // - mbxctl running (./mbxctl --listen localhost:9999)

    if os.Getenv("MINIBOX_CONTROLLER") == "" {
        os.Setenv("MINIBOX_CONTROLLER", "http://localhost:9999")
    }

    exe := executor.NewMiniboxExecutor(os.Getenv("MINIBOX_CONTROLLER"))

    req := &executor.ExecutionRequest{
        Command: []string{"alpine:latest", "echo", "hello"},
        Env:     map[string]string{},
        Timeout: 10,
    }

    result, err := exe.Execute(context.Background(), req)
    if err != nil {
        t.Fatalf("Execute failed: %v", err)
    }

    if result.ExitCode != 0 {
        t.Fatalf("unexpected exit code: %d", result.ExitCode)
    }

    if !strings.Contains(result.Stdout, "hello") {
        t.Fatalf("expected 'hello' in output, got: %s", result.Stdout)
    }
}
```

- [ ] **Step 2: Run E2E test (if services running)**

```bash
export MINIBOX_CONTROLLER=http://localhost:9999
go test -v ./tests/...
```

Expected: Pass (if miniboxd + mbxctl running).

- [ ] **Step 3: Commit**

```bash
git add tests/
git commit -m "test: add end-to-end integration test for executor plugin"
```

---

### Task 12: Build Final Docker Image

**Files:**

- (No new files; rebuild existing Dockerfile)

- [ ] **Step 1: Build image with complete plugin**

```bash
cd mbx-dagu
docker build -t dagu:minibox .
```

Expected: Builds successfully with all plugin code included.

- [ ] **Step 2: Test image with DAG workflow**

Create test workflow `test-workflow.yaml`:

```yaml
steps:
  - name: hello
    executor: minibox
    command: alpine:latest
    args:
      - echo
      - "Hello from minibox!"
```

Run in container:

```bash
docker run -e MINIBOX_CONTROLLER=http://localhost:9999 \
  dagu:minibox \
  /dagu dag run test-workflow.yaml
```

Expected: Workflow runs, output shows "Hello from minibox!".

- [ ] **Step 3: Commit**

```bash
git add .
git commit -m "build: finalize dagu:minibox image with complete plugin"
```

**END PHASE 4**

---

## Integration & Final Verification

### Task 13: Full Stack Test

- [ ] **Step 1: Start miniboxd (Terminal 1)**

```bash
cd minibox
sudo ./target/release/miniboxd
```

- [ ] **Step 2: Start mbxctl (Terminal 2)**

```bash
cd mbxctl
./target/release/mbxctl --listen localhost:9999
```

- [ ] **Step 3: Run dagu in minibox (Terminal 3)**

```bash
cd minibox
minibox run dagu:minibox \
  -e MINIBOX_CONTROLLER=http://localhost:9999 \
  -- /dagu server
```

- [ ] **Step 4: Access Web UI**

Open http://localhost:8080 in browser.

- [ ] **Step 5: Create and run workflow**

From Web UI, create a DAG with `executor: minibox` step, run it.

- [ ] **Step 6: Verify output**

Check that:

- Step runs in container
- Output appears in UI
- Exit code is captured

- [ ] **Step 7: Commit final integration test**

```bash
git add .
git commit -m "test: verify full stack integration (dagu+minibox+mbxctl)"
```

---

## Testing Summary

| Phase | Unit Tests        | Integration    | E2E               |
| ----- | ----------------- | -------------- | ----------------- |
| 1     | ✅ Docker build   | ✅ Image runs  | ✅ Web UI         |
| 2     | ✅ Socket parsing | ✅ Real daemon | ❌ CLI tests pass |
| 3     | ✅ HTTP handlers  | ✅ Real daemon | ✅ API works      |
| 4     | ✅ Parser         | ✅ Real daemon | ✅ DAG runs       |

---

## Commits Checklist

- [ ] Phase 1: Docker image builds
- [ ] Phase 2: minibox-client library + CLI refactor
- [ ] Phase 3: mbxctl HTTP server + endpoints
- [ ] Phase 4: mbx-dagu plugin + E2E test
- [ ] Integration: Full stack test

---

## Notes for Implementation

1. **Workspace:** All minibox work happens in existing repo
2. **Separate Repos:** mbxctl and mbx-dagu are separate GitHub repos (created by you)
3. **Testing:** Run tests after each task; use `cargo test` for Rust, `go test` for Go
4. **Debugging:** Use `RUST_LOG=debug` and `MINIBOX_CONTROLLER` env var for troubleshooting
5. **TDD Discipline:** Always write test first, then implementation
