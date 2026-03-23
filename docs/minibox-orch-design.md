---
status: future
note: Design for minibox-orch crate (self-evolving agent orchestrator). Not started. Prerequisite: exec/logs/named-container gaps in minibox must be closed first (see docs/plans/maestro-minibox.md).
---

# minibox-orch Design

A new Rust binary crate that treats `miniboxd` as a substrate and layers self-evolving agents on top.

## High-level Shape

- New crate: `minibox-orch` (binary) in the workspace.
- Responsibility:
  - Talk to `miniboxd` via JSON-over-Unix protocol.
  - Schedule/evaluate jobs inside containers.
  - Run inner task loops and outer harness-evolution loops.
- Keep `linuxbox` strictly infra (images, containers, adapters); all agent logic lives in `minibox-orch`.

## Architecture

Hexagonal (ports and adapters), matching the patterns established in `linuxbox`.

```text
                    Composition Root (main.rs)
                    Wires adapters, injects deps
                              |
            +-----------------+------------------+
            |                                    |
    +-------v---------+              +-----------v-----------+
    |  Domain Layer   |              |  Infrastructure       |
    |                 |              |     Adapters          |
    |  Traits (Ports) |              |                       |
    |  - DaemonClient -+------+-------+-> SocketDaemonClient |
    |  - ModelClient  -+-----+-------+-> AnthropicModel      |
    |  - ProfileStore -+-----+-------+-> OpenAiModel         |
    |  - TelemetryStr -+-----+-------+-> TomlProfileStore    |
    |  - ApprovalGate -+-----+-------+-> SqliteTelemetry     |
    |                 |      |       |   TerminalApproval    |
    |  Domain Types   |      |       +-----------------------+
    |  Domain Errors  |      |
    |  SafetyPolicy   |      +-- Mocks (for testing)
    +-----------------+
            ^
            |
    +-------+---------+
    |  Business Logic  |
    |  inner_loop.rs   |
    |  outer_loop.rs   |
    +------------------+
```

Dependencies point inward. Business logic depends only on domain traits, never on adapters.

## Module Structure

```text
crates/minibox-orch/
  Cargo.toml
  src/
    main.rs                    # Composition root + CLI (clap)
    lib.rs                     # Re-exports for testing
    domain.rs                  # Traits (ports) + types + errors
    domain/
      safety.rs                # SafetyPolicy, HarnessDiff validation
    inner_loop.rs              # TaskRunner business logic
    outer_loop.rs              # HarnessEvolver business logic
    adapters/
      mod.rs                   # Re-exports
      daemon_client.rs         # DaemonClient via Unix socket
      anthropic_model.rs       # ModelClient via Anthropic API
      openai_model.rs          # ModelClient via OpenAI-compatible API
      toml_profile_store.rs    # ProfileStore backed by TOML files
      sqlite_telemetry.rs      # TelemetryStore via rusqlite
      terminal_approval.rs     # ApprovalGate via stdin prompts
      mocks.rs                 # Mock implementations for all traits
```

## Domain Layer

### Traits (Ports)

Five ports. All extend `AsAny + Send + Sync`. Async traits use `#[async_trait]`.

#### DaemonClient

Talks to `miniboxd` over its Unix socket protocol.

```rust
#[async_trait]
pub trait DaemonClient: AsAny + Send + Sync {
    async fn pull_image(&self, image: &str, tag: &str) -> Result<()>;
    async fn run_container(&self, spec: &JobSpec) -> Result<ContainerHandle>;
    async fn wait_container(&self, handle: &ContainerHandle) -> Result<ExitStatus>;
    async fn stop_container(&self, handle: &ContainerHandle) -> Result<()>;
    async fn list_containers(&self) -> Result<Vec<ContainerSnapshot>>;
}
```

#### ModelClient

LLM inference -- chat completions with optional structured output.

```rust
#[async_trait]
pub trait ModelClient: AsAny + Send + Sync {
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse>;
}
```

#### ProfileStore

Versioned `HarnessProfile` persistence.

```rust
#[async_trait]
pub trait ProfileStore: AsAny + Send + Sync {
    async fn load(&self, id: &ProfileId) -> Result<Option<HarnessProfile>>;
    async fn save(&self, profile: &HarnessProfile) -> Result<()>;
    async fn list(&self) -> Result<Vec<ProfileId>>;
    async fn load_active(&self) -> Result<HarnessProfile>;
    async fn set_active(&self, id: &ProfileId) -> Result<()>;
}
```

#### TelemetryStore

Write and query `JobRun` metrics.

```rust
#[async_trait]
pub trait TelemetryStore: AsAny + Send + Sync {
    async fn record_run(&self, run: &JobRun) -> Result<()>;
    async fn query_runs(&self, filter: &RunFilter) -> Result<Vec<JobRun>>;
    async fn aggregate_metrics(&self, filter: &RunFilter) -> Result<RunMetrics>;
}
```

#### ApprovalGate

Human-in-the-loop for sensitive profile changes.

```rust
#[async_trait]
pub trait ApprovalGate: AsAny + Send + Sync {
    async fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalDecision>;
}
```

### Dyn Type Aliases

```rust
pub type DynDaemonClient = Arc<dyn DaemonClient>;
pub type DynModelClient = Arc<dyn ModelClient>;
pub type DynProfileStore = Arc<dyn ProfileStore>;
pub type DynTelemetryStore = Arc<dyn TelemetryStore>;
pub type DynApprovalGate = Arc<dyn ApprovalGate>;
```

### Domain Types

#### Job Execution

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub family: String,                    // e.g. "build-backend", "run-e2e-tests"
    pub image: String,
    pub tag: String,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
    pub resource_hints: ResourceHints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceHints {
    pub memory_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub max_runtime_secs: Option<u64>,
    pub pids_max: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ContainerHandle {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub killed_by_signal: Option<String>,  // "OOM", "SIGKILL", etc.
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSnapshot {
    pub id: String,
    pub image: String,
    pub state: String,
}
```

#### Telemetry

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub id: String,
    pub family: String,
    pub profile_id: String,
    pub spec: JobSpec,
    pub exit_status: ExitStatus,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub peak_memory_bytes: Option<u64>,
    pub cpu_time_ms: Option<u64>,
    pub log_snippet: String,
}

#[derive(Debug, Clone)]
pub struct RunFilter {
    pub family: Option<String>,
    pub profile_id: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetrics {
    pub total_runs: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub oom_count: usize,
    pub timeout_count: usize,
    pub p50_duration_ms: u64,
    pub p95_duration_ms: u64,
    pub avg_peak_memory_bytes: u64,
}
```

#### Profiles

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessProfile {
    pub id: ProfileId,
    pub description: String,
    pub version: u32,
    pub parent_id: Option<ProfileId>,
    pub created_at: String,
    pub default_limits: ResourceHints,
    pub safety: SafetyConfig,
    pub scheduling: SchedulingConfig,
    pub per_job_overrides: HashMap<String, ResourceHints>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyConfig {
    pub network_mode: NetworkMode,
    pub syscall_profile: SyscallProfile,
    pub max_runtime_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkMode {
    None,
    Bridge,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyscallProfile {
    Strict,
    Default,
    Permissive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulingConfig {
    pub max_parallel_jobs: usize,
    pub max_attempts_per_task: usize,
    pub backoff_initial_ms: u64,
    pub backoff_multiplier: f64,
}
```

#### HarnessDiff (Constrained Patch)

The model emits these. Each variant represents one allowed change.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HarnessDiff {
    SetMemoryBytes { family: Option<String>, value: u64 },
    SetCpuWeight { family: Option<String>, value: u64 },
    SetMaxRuntime { family: Option<String>, secs: u64 },
    SetNetworkMode(NetworkMode),
    SetSyscallProfile(SyscallProfile),
    SetMaxParallelJobs(usize),
    SetMaxAttempts(usize),
    SetBackoff { initial_ms: u64, multiplier: f64 },
}
```

#### Model Interaction

```rust
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub max_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Role { System, User, Assistant }

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}
```

#### Approval

```rust
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub description: String,
    pub diff: HarnessDiff,
    pub current_value: String,
    pub proposed_value: String,
}

#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    Approved,
    Denied { reason: String },
}
```

#### Job Memory

Short-term memory log per job family.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMemory {
    pub family: String,
    pub entries: Vec<MemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub run_id: String,
    pub config_summary: String,
    pub outcome_summary: String,
    pub model_critique: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
```

### Safety Validation

Two layers:

**Domain layer** (`domain/safety.rs`) -- structural constraints, pure logic:

```rust
pub struct SafetyPolicy {
    pub max_memory_bytes: u64,       // hard cap, e.g. 32 GB
    pub max_cpu_weight: u64,         // hard cap, e.g. 10000
    pub max_runtime_secs: u64,       // hard cap, e.g. 3600
    pub max_parallel_jobs: usize,    // hard cap, e.g. 64
}

impl HarnessDiff {
    /// Validate against hard caps. Returns Err(SafetyViolation) if out of bounds.
    pub fn validate(&self, policy: &SafetyPolicy) -> Result<(), OrchestratorError>;

    /// Does this diff require human approval?
    pub fn requires_approval(&self) -> bool;

    /// Apply this diff to a profile, producing a new versioned profile.
    pub fn apply(&self, profile: &HarnessProfile) -> HarnessProfile;
}
```

`requires_approval()` returns `true` for:

- `SetNetworkMode(Host)` -- escalation to host networking
- `SetSyscallProfile(Permissive)` -- weakening syscall restrictions

**Middleware layer** (in `outer_loop.rs`) -- async approval I/O:

The outer loop checks `requires_approval()` and calls `ApprovalGate` before applying.
This keeps the async human-in-the-loop concern out of the domain types.

### Domain Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("daemon communication failed: {0}")]
    DaemonError(String),

    #[error("model inference failed: {0}")]
    ModelError(String),

    #[error("profile '{0}' not found")]
    ProfileNotFound(String),

    #[error("safety violation: {0}")]
    SafetyViolation(String),

    #[error("approval denied: {0}")]
    ApprovalDenied(String),

    #[error("budget exhausted for job family '{0}'")]
    BudgetExhausted(String),

    #[error(transparent)]
    Infrastructure(#[from] anyhow::Error),
}
```

## Inner Loop: Task-level Self-improvement

Per job family (e.g., "build-backend", "run-e2e-tests"):

```rust
pub struct TaskRunner {
    daemon: DynDaemonClient,
    model: DynModelClient,
    telemetry: DynTelemetryStore,
    profiles: DynProfileStore,
}
```

### Algorithm

1. Load active `HarnessProfile` from `ProfileStore`.
2. Resolve resource limits: `per_job_overrides[family]` with `default_limits` fallback.
3. `DaemonClient::pull_image()` if needed.
4. `DaemonClient::run_container(spec)` with resolved limits.
5. `DaemonClient::wait_container(handle)` -- collect `ExitStatus`.
6. Build `JobRun` with timing, exit code, peak memory, log snippet.
7. `TelemetryStore::record_run()`.
8. If failed and attempts remain:
   - Serialize recent `JobMemory` entries + current run into a prompt.
   - `ModelClient::complete()` -- ask for self-critique and suggestions.
   - Append `MemoryEntry` to `JobMemory`.
   - Apply suggestions to `JobSpec` (within profile limits), retry from step 3.
9. Return all `JobRun`s for this session.

### Budget Controls

- `scheduling.max_attempts_per_task` -- max retries.
- `safety.max_runtime_secs` -- wall-clock timeout per attempt.
- Exponential backoff between retries: `backoff_initial_ms * backoff_multiplier^attempt`.

## Outer Loop: Harness Self-evolution

```rust
pub struct HarnessEvolver {
    daemon: DynDaemonClient,
    model: DynModelClient,
    telemetry: DynTelemetryStore,
    profiles: DynProfileStore,
    approval: DynApprovalGate,
    policy: SafetyPolicy,
}
```

### Algorithm

1. Load active `HarnessProfile`.
2. `TelemetryStore::aggregate_metrics()` for recent runs under this profile.
3. Build prompt with metrics summary (success rate, p95 latency, OOM rate, etc.).
4. `ModelClient::complete()` -- ask meta-agent to emit a `HarnessDiff` JSON.
5. Parse and validate:
   - `HarnessDiff::validate(&self.policy)` -- reject if exceeds hard caps.
   - `HarnessDiff::requires_approval()` -- call `ApprovalGate` if needed.
6. `HarnessDiff::apply(profile)` -- create candidate profile with incremented version.
7. `ProfileStore::save(candidate)`.
8. Run fixed eval suite under candidate profile via inner loop.
9. `TelemetryStore::aggregate_metrics()` for candidate runs.
10. Compare candidate vs baseline:
    - If candidate wins on primary objective, `ProfileStore::set_active(candidate.id)`.
    - Otherwise discard candidate.

### Evolution Outcome

```rust
pub enum EvolutionOutcome {
    Promoted {
        old_id: ProfileId,
        new_id: ProfileId,
        improvement: RunMetrics,
    },
    Discarded {
        reason: String,
    },
    Blocked {
        pending_approval: ApprovalRequest,
    },
}
```

## Adapters

| Adapter                | Trait            | Backend                                      |
| ---------------------- | ---------------- | -------------------------------------------- |
| `SocketDaemonClient`   | `DaemonClient`   | Unix socket, reuses `linuxbox::protocol`  |
| `AnthropicModelClient` | `ModelClient`    | `reqwest` to `api.anthropic.com/v1/messages` |
| `OpenAiModelClient`    | `ModelClient`    | `reqwest` to configurable base URL           |
| `TomlProfileStore`     | `ProfileStore`   | TOML files in `profiles/<id>.toml`           |
| `SqliteTelemetryStore` | `TelemetryStore` | `rusqlite` with `job_runs` table             |
| `TerminalApprovalGate` | `ApprovalGate`   | stdin/stdout prompts                         |

### SocketDaemonClient

Reuses the `send_request` pattern from `minibox-cli/src/commands/mod.rs`. Connects to
`MINIBOX_SOCKET_PATH` (default `/run/minibox/miniboxd.sock`), sends `DaemonRequest` JSON lines,
reads `DaemonResponse` JSON lines. Maps response variants to domain method returns.

`wait_container` polls `list_containers` every 500 ms until container state is `Stopped` or
`Failed`, or timeout expires.

### AnthropicModelClient

POST to `https://api.anthropic.com/v1/messages` with `x-api-key` header. Reads
`ANTHROPIC_API_KEY` from environment. Configurable model name (default `claude-sonnet-4-20250514`).

### OpenAiModelClient

POST to `{base_url}/v1/chat/completions`. Reads `OPENAI_API_KEY` and optional
`OPENAI_BASE_URL` (default `https://api.openai.com`). Works with Ollama by setting
`OPENAI_BASE_URL=http://localhost:11434`.

### TomlProfileStore

Directory-based. Each profile at `{profiles_dir}/{id}.toml`. Active profile tracked in
`{profiles_dir}/active.txt`. Uses `toml::from_str` / `toml::to_string_pretty`.

### SqliteTelemetryStore

Single SQLite database at `{data_dir}/telemetry.db`. Schema:

```sql
CREATE TABLE IF NOT EXISTS job_runs (
    id TEXT PRIMARY KEY,
    family TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    spec_json TEXT NOT NULL,
    exit_code INTEGER,
    killed_by TEXT,
    duration_ms INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    peak_memory_bytes INTEGER,
    cpu_time_ms INTEGER,
    log_snippet TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runs_family ON job_runs(family);
CREATE INDEX IF NOT EXISTS idx_runs_profile ON job_runs(profile_id);
CREATE INDEX IF NOT EXISTS idx_runs_started ON job_runs(started_at);
```

### TerminalApprovalGate

Prints the proposed change to stdout, reads `y/n` from stdin. Uses
`tokio::task::spawn_blocking` for the blocking I/O (matching the async/sync boundary
pattern in `miniboxd/src/handler.rs`).

### Mock Adapters

Follow the pattern from `linuxbox/src/adapters/mocks.rs`:

- `Arc<Mutex<State>>` for interior mutability.
- Builder methods: `.with_run_failure()`, `.with_active_profile()`, `.with_response()`.
- Call-count accessors: `.run_count()`, `.complete_count()`.
- Hand-written `impl AsAny for MockFoo` (own `AsAny` trait, not from minibox-macros).

Five mock types: `MockDaemonClient`, `MockModelClient`, `MockProfileStore`,
`MockTelemetryStore`, `MockApprovalGate`.

## Composition Root

CLI via clap derive, four subcommands:

```text
minibox-orch run     <family> [--max-attempts N]  # Inner loop for one job family
minibox-orch evolve  <eval-suite-path>            # Outer evolution loop
minibox-orch init    [--description TEXT]          # Create default profile
minibox-orch stats   [--family NAME]              # Query telemetry
minibox-orch profile [--id ID]                    # Show profile details
```

Environment variables:

- `MINIBOX_SOCKET_PATH` -- daemon socket (default `/run/minibox/miniboxd.sock`)
- `MINIBOX_ORCH_PROFILES_DIR` -- profile storage (default `./profiles`)
- `MINIBOX_ORCH_DATA_DIR` -- telemetry DB location (default `./data`)
- `ANTHROPIC_API_KEY` -- for Anthropic adapter
- `OPENAI_API_KEY` -- for OpenAI adapter
- `OPENAI_BASE_URL` -- for OpenAI-compatible endpoints
- `MINIBOX_ORCH_MODEL` -- which adapter to use: `anthropic` (default), `openai`

## Dependencies

```text
minibox-orch
  +-- linuxbox       (protocol types: DaemonRequest, DaemonResponse, ContainerInfo)
  +-- serde, serde_json (serialization)
  +-- toml              (profile serialization)
  +-- rusqlite          (telemetry storage)
  +-- tokio             (async runtime)
  +-- anyhow, thiserror (error handling)
  +-- tracing           (structured logging)
  +-- clap              (CLI)
  +-- chrono            (timestamps)
  +-- uuid              (run/profile IDs)
  +-- reqwest           (HTTP for model adapters)
  +-- async-trait       (async trait support)
```

No `nix`/`libc` -- this crate is a pure client. Compiles on macOS.

## Process Topology

```text
miniboxd (root)                minibox-orch (unprivileged)
  |                                  |
  | <--- Unix socket protocol -----> |
  |    DaemonRequest/Response        |
  |                                  |---> Anthropic/OpenAI API
  |                                  |---> profiles/*.toml
  |                                  |---> data/telemetry.db
  |                                  |---> stdin/stdout (approval)
```
