# Design: MCP exec/events tools, run-minibox exec step, minibox-tui read-only indicator

## Goal

Three independent Quick Wins from a gm-ideate pass: (1) the `minibox-mcp` server is missing
`exec` and event-visibility tools even though the daemon already supports both; (2) the
`run-minibox` skill never exercises `mbx exec` despite it being a real, documented capability;
(3) `minibox-tui` doesn't tell the user it's read-only, even though that's a deliberate design
choice recorded in `.ctx/memory-bank/progress.mbx.md`.

## Approved Approach

For each, the smallest shape that reuses an existing precedent in the same file/crate rather
than introducing a new pattern. Settled during brainstorm; adjusted once during design (see
"Design Correction" below).

---

## Part 1: `minibox_exec` + `minibox_events` MCP tools

### Crate Ownership

- **Owner crate**: `crates/mcp` (`minibox-mcp`) — it already owns every other `minibox_*` tool.
- **Affected crates**: none beyond `minibox-mcp` and its existing dependency on
  `minibox-core` (for `DaemonRequest::Exec`, `DaemonRequest::SubscribeEvents`,
  `minibox_core::events::ContainerEvent`, already a transitive dependency via
  `minibox-domain`).

### Design Correction (from brainstorm)

Brainstorm proposed reusing `minibox_core::events::ContainerEvent` directly as the MCP output
type. Context-map review found two problems: `ContainerEvent` has no `#[derive(JsonSchema)]`
(every other `*Output` type in `crates/mcp/src/types.rs` does), and its `timestamp: SystemTime`
field has no schema-friendly representation. Fix: a local schema-facing mirror type,
`ContainerEventOutput`, following the exact precedent already in `types.rs` — the comment
`"Local schema-facing mirror of ContainerInfo"` above `ContainerInfoOutput`. This keeps
`minibox-domain` free of MCP-specific derives, per this repo's hexagonal convention (domain
crates have zero adapter-facing dependencies — see `AGENTS.md`'s Architecture Guardrails).

### Public API

#### Types (new, in `crates/mcp/src/types.rs`)

```rust
/// Input for `minibox_exec`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExecContainerInput {
    /// Container ID or name. Must be in the Running state.
    pub id: String,
    /// Command and arguments to execute.
    pub cmd: Vec<String>,
    /// Additional environment variables in `KEY=VALUE` form.
    #[serde(default)]
    pub env: Vec<String>,
    /// Working directory inside the container. Defaults to `/`.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Run the exec process as a specific user (e.g. `"nobody"`, `"1000:1000"`).
    #[serde(default)]
    pub user: Option<String>,
}

/// Output for `minibox_exec`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecContainerOutput {
    /// Decoded stdout.
    pub stdout: String,
    /// Decoded stderr.
    pub stderr: String,
    /// Exit code from the exec process.
    pub exit_code: Option<i32>,
    /// Whether decoded output was truncated.
    pub truncated: bool,
}

/// Input for `minibox_events`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct EventsInput {
    /// How long to collect events before returning, in milliseconds.
    /// Defaults to 2000, clamped to the 1..=10_000 range.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// Local schema-facing mirror of `ContainerEvent`, flattened for MCP output.
/// Exactly one event kind's fields are populated per instance; the rest are `None`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ContainerEventOutput {
    /// Event kind: `created`, `started`, `stopped`, `paused`, `resumed`,
    /// `oom_killed`, `image_pulled`, `image_removed`, or `image_pruned`.
    pub kind: String,
    /// Container ID, for container-scoped event kinds.
    pub id: Option<String>,
    /// Image reference, for `created`/`image_pulled`/`image_removed` events.
    pub image: Option<String>,
    /// Host process ID, for `started` events.
    pub pid: Option<u32>,
    /// Exit code, for `stopped` events.
    pub exit_code: Option<i32>,
    /// Downloaded size in bytes, for `image_pulled` events.
    pub size_bytes: Option<u64>,
    /// Number of images removed, for `image_pruned` events.
    pub count: Option<usize>,
    /// Bytes freed, for `image_pruned` events.
    pub freed_bytes: Option<u64>,
    /// Event time as milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
}

/// Output for `minibox_events`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventsOutput {
    /// Events collected during the window, in daemon order.
    pub events: Vec<ContainerEventOutput>,
    /// Number of events collected.
    pub count: usize,
}
```

#### Conversion (new, in `crates/mcp/src/types.rs`, alongside the existing

`impl From<minibox_core::protocol::ContainerInfo> for ContainerInfoOutput`)

```rust
impl From<minibox_core::events::ContainerEvent> for ContainerEventOutput;
```

Signature only — the body is a `match` over all nine `ContainerEvent` variants, each arm
setting `kind` to the matching snake_case string and populating only the fields that variant
carries; `timestamp_ms` comes from
`timestamp.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64`
in every arm.

#### Client method (new, in `crates/mcp/src/client.rs`)

```rust
impl MiniboxDaemonClient {
    /// Send a request and collect daemon responses for a bounded time window,
    /// closing the subscription when the window elapses rather than waiting
    /// for a terminal response (which `SubscribeEvents` never sends).
    ///
    /// # Errors
    ///
    /// Returns an error if the socket call, response decoding, or output
    /// accounting fails.
    pub async fn call_for_duration(
        &self,
        request: DaemonRequest,
        max_output_bytes: usize,
        duration: std::time::Duration,
    ) -> Result<DaemonCallResult>;
}
```

Additive only — no change to `call`, `call_limited`, `call_authorized`, or `call_raw`'s
existing signatures or behavior.

#### Tool functions (new, in `crates/mcp/src/tools/containers.rs`)

```rust
/// Execute a command inside a running container.
///
/// # Errors
///
/// Returns an error if policy denies the operation, input is invalid, or the daemon call fails.
pub async fn exec(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: ExecContainerInput,
) -> Result<ExecContainerOutput>;
```

Plus a private `normalize_exec_output(responses: Vec<DaemonResponse>, max_output_bytes: usize, output_truncated: bool) -> Result<ExecContainerOutput>` — a deliberate near-copy of the
existing `normalize_run_output` (same file, same loop shape) with the `ContainerCreated` arm
removed, kept as a separate function per the explicit decision not to generalize the two
(Run and Exec output handling may diverge later).

#### Tool function (new file: `crates/mcp/src/tools/events.rs`)

```rust
//! Event-stream MCP tool implementation.

/// Collect container lifecycle events for a bounded time window.
///
/// # Errors
///
/// Returns an error if the daemon call fails.
pub async fn events(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: EventsInput,
) -> Result<EventsOutput>;
```

Clamps `input.duration_ms.unwrap_or(2000)` to `1..=10_000` before use. No policy gate call —
`SubscribeEvents` is already in `client.rs::ensure_read_only`'s allowlist.

#### Server registration (edits to `crates/mcp/src/server.rs`)

```rust
#[tool(name = "minibox_exec", description = "Execute a command inside a running container")]
pub async fn minibox_exec(
    &self,
    input: Parameters<ExecContainerInput>,
) -> Result<Json<ExecContainerOutput>, McpError>;

#[tool(name = "minibox_events", description = "Collect recent container lifecycle events")]
pub async fn minibox_events(
    &self,
    input: Parameters<EventsInput>,
) -> Result<Json<EventsOutput>, McpError>;
```

Both follow the exact `tracing::debug!` + delegate-to-`tools::` module pattern every existing
tool method uses (see `minibox_doctor` at `server.rs:93`).

Add `mod events;` to `crates/mcp/src/tools/mod.rs` (or wherever the module is declared — the
existing `use crate::tools::{containers, doctor, images};` import in `server.rs` becomes
`use crate::tools::{containers, doctor, events, images};`).

### Data Flow

**`minibox_exec`:**

1. Source: MCP client call → `ExecContainerInput`.
2. Transform: `policy.authorize_mutation("minibox_exec", DaemonRequest::Exec { .. })` (gate) →
   `client.call_authorized(..)` (collect `ContainerOutput`/`ContainerStopped` responses) →
   `normalize_exec_output(..)` (decode base64, truncate to `policy.max_output_bytes`).
3. Sink: `ExecContainerOutput` returned as `Json<..>`.

**`minibox_events`:**

1. Source: MCP client call → `EventsInput`.
2. Transform: clamp `duration_ms` → `client.call_for_duration(DaemonRequest::SubscribeEvents, ..)`
   (collect `DaemonResponse::Event` until the window elapses) → map each `ContainerEvent` through
   `ContainerEventOutput::from`.
3. Sink: `EventsOutput` returned as `Json<..>`.

### Hexagonal Boundaries

No new ports or adapters — `minibox-mcp` is itself the adapter layer over the daemon's existing
`DaemonClient` port (`minibox_core::client::DaemonClient`). `ContainerEventOutput` is a
presentation-layer mirror, not a new port.

### Integration Points

- No feature flag — matches every other tool in this crate.
- No existing public API changes; `call_for_duration` is additive on `MiniboxDaemonClient`.
- `minibox_exec` policy gate reuses `AgentPolicy::authorize_mutation`, already public.

### Out of Scope

- PTY/interactive exec (`-it` passthrough), `SendInput`/`ResizePty` wiring.
- A true unbounded `SubscribeEvents` stream (MCP's tool-call model is request/response).
- Any `minibox-core::protocol` change.
- Any change to `minibox-domain::events::ContainerEvent` itself.

### Risk

- [ ] Breaking API changes: no.
- [ ] New external dependency: no.
- [ ] Feature flag required: no.

---

## Part 2: `run-minibox` skill exercises `exec`

### Crate Ownership

Not a crate — `.claude/skills/run-minibox/` (`driver.nu` + `SKILL.md`). No Rust changes.

### Public API

New exported Nushell command in `driver.nu`, matching the existing `logs`/`stop`/
`rm-container` pattern exactly:

```nu
export def exec [target: string, ...cmd: string] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) exec $target -- ...$cmd
}
```

`SKILL.md` documents a new flow using existing primitives (no change to `smoke`, whose
container is ephemeral/auto-removed and has nothing left running to exec into):

```
nu driver.nu run-cmd alpine:latest sleep 30   # detached, named, non-ephemeral
nu driver.nu exec <name> echo hello-from-exec
nu driver.nu stop <name>; nu driver.nu rm-container <name>
```

### Out of Scope

- Changing `smoke()`'s contract.
- Any Rust change.

### Risk

- [ ] Breaking API changes: no.
- [ ] New external dependency: no.
- [ ] Feature flag required: no.

---

## Part 3: `minibox-tui` read-only indicator

### Crate Ownership

`crates/minibox-tui`.

### Public API

No new public items — a one-line edit inside the existing private `draw_status_bar` function
in `crates/minibox-tui/src/ui.rs` (non-error branch only):

```rust
Line::from(" q: quit  |  j/k, ↑/↓: select  |  read-only — live ps + events ")
```

### Out of Scope

- The error branch (unchanged).
- The containers panel title.
- Any new widget or layout constraint.

### Risk

- [ ] Breaking API changes: no.
- [ ] New external dependency: no.
- [ ] Feature flag required: no.

---

## Sequencing

All three are independent — no shared touch points beyond Part 1 and Part 3 both reading
`ContainerEvent`-shaped data, which don't interact (Part 1 converts to `ContainerEventOutput`
for MCP's schema needs; Part 3 only edits a static string, no event data involved). Suggested
order: Part 3 (smallest) → Part 2 (no Rust, but depends on nothing) → Part 1 (largest, most
test surface). Can also run in parallel across three commits with no merge conflicts expected
(three different crates/dirs).
