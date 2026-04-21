# Quick Wins Implementation Plan

# Container Freeze, Events, Image GC + Leases, Bridge Networking

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four independently committable features — container pause/resume, a pub/sub event broker, image garbage collection with lease protection, and bridge networking — all following minibox's existing hexagonal architecture patterns.

**Architecture:** Each feature adds a new port (trait) in `minibox-core`, a concrete adapter in `minibox` or `daemonbox`, protocol variants in both `protocol.rs` files, handler arms in `daemonbox/src/handler.rs`, and CLI subcommands in `minibox-cli`. Composition roots (`miniboxd/src/main.rs`) wire adapters in. All Linux-only features are gated with `#[cfg(target_os = "linux")]`.

**Tech Stack:** Rust 2024 edition, `async_trait`, `nix` crate, `tokio::sync::broadcast` for events, `ipnet` crate for CIDR math, `iptables` subprocess for NAT, existing `CgroupManager` for freeze.

**Spec:** `docs/superpowers/specs/2026-04-02-quick-wins-design.md`

---

## File Map

| File                                            | Action | Purpose                                                                                                                                      |
| ----------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/minibox-core/src/events.rs`             | Create | `ContainerEvent` enum, `EventSink` + `EventSource` traits, `BroadcastEventBroker`                                                            |
| `crates/minibox-core/src/image/lease.rs`        | Create | `LeaseRecord`, `DiskLeaseService`, `ImageLeaseService` trait                                                                                 |
| `crates/minibox-core/src/image/gc.rs`           | Create | `ImageGc`, `ImageGarbageCollector` trait, `PruneReport`                                                                                      |
| `crates/minibox/src/adapters/network/bridge.rs` | Create | `BridgeNetwork`, `IpAllocator` (Linux-only)                                                                                                  |
| `crates/minibox-core/src/lib.rs`                | Modify | `pub mod events;`                                                                                                                            |
| `crates/minibox-core/src/domain.rs`             | Modify | `DynEventSink`, `DynEventSource` type aliases                                                                                                |
| `crates/minibox-core/src/image/mod.rs`          | Modify | `list_all_images()`, `delete_image()`, `image_size_bytes()`                                                                                  |
| `crates/minibox-core/src/protocol.rs`           | Modify | `PauseContainer`, `ResumeContainer`, `Prune`, `SubscribeEvents` requests; `ContainerPaused`, `ContainerResumed`, `Pruned`, `Event` responses |
| `crates/minibox/src/protocol.rs`                | Modify | Mirror all protocol changes                                                                                                                  |
| `crates/minibox/src/container/cgroups.rs`       | Modify | `pause()`, `resume()` methods on `CgroupManager`                                                                                             |
| `crates/minibox/src/adapters/network/mod.rs`    | Modify | `pub mod bridge;` + re-export (cfg linux)                                                                                                    |
| `crates/daemonbox/src/handler.rs`               | Modify | `event_sink` field; `handle_pause`, `handle_resume`, `handle_prune`, `handle_subscribe_events`; emit events at existing lifecycle points     |
| `crates/daemonbox/src/server.rs`                | Modify | Dispatch arms for 4 new requests; `Event` and `ContainerPaused`/`ContainerResumed` in `is_terminal_response`                                 |
| `crates/daemonbox/src/state.rs`                 | Modify | `ContainerState::Paused`; allow `Running→Paused` and `Paused→Running` transitions; `allocated_ips: HashMap<String, IpAddr>` field            |
| `crates/miniboxd/src/main.rs`                   | Modify | Wire `BroadcastEventBroker`; wire `BridgeNetwork` for bridge mode                                                                            |
| `crates/minibox-cli/src/commands/pause.rs`      | Create | `minibox pause <id>`                                                                                                                         |
| `crates/minibox-cli/src/commands/resume.rs`     | Create | `minibox resume <id>`                                                                                                                        |
| `crates/minibox-cli/src/commands/events.rs`     | Create | `minibox events` (streaming JSON-lines)                                                                                                      |
| `crates/minibox-cli/src/commands/prune.rs`      | Create | `minibox prune [--dry-run]`                                                                                                                  |
| `crates/minibox-cli/src/commands/rmi.rs`        | Create | `minibox rmi <image>`                                                                                                                        |
| `crates/minibox-cli/src/main.rs`                | Modify | Register 5 new subcommands                                                                                                                   |

---

## Phase 1: Container Freeze / Pause

### Task 1: Add `pause()` and `resume()` to `CgroupManager`

**Files:**

- Modify: `crates/minibox/src/container/cgroups.rs`

- [ ] **Step 1: Write failing compile test**

Add at the bottom of `crates/minibox/src/container/cgroups.rs` inside the existing `#[cfg(test)] mod tests`:

```rust
#[cfg(target_os = "linux")]
#[test]
fn test_cgroup_pause_resume_methods_exist() {
    // Verify the methods compile. Real behavior requires root + cgroup mount.
    let mgr = CgroupManager::new("test-pause-id", CgroupConfig::default());
    let _ = mgr.cgroup_path(); // confirm path helper exists
    // pause/resume are async; just confirm they exist via reference
    let _: fn(&CgroupManager) -> _ = CgroupManager::pause;
    let _: fn(&CgroupManager) -> _ = CgroupManager::resume;
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo check -p minibox 2>&1 | grep "no method named \`pause\`"
```

Expected: error about missing `pause` method.

- [ ] **Step 3: Add `cgroup_path()` helper and `pause()`/`resume()` to `CgroupManager`**

In `crates/minibox/src/container/cgroups.rs`, add after the `cleanup()` method:

```rust
/// Returns the cgroup path for this container.
pub fn cgroup_path(&self) -> &Path {
    &self.path
}

/// Freeze all processes in this container's cgroup.
///
/// Writes `"1"` to `cgroup.freeze`. Requires cgroups v2.
pub async fn pause(&self) -> anyhow::Result<()> {
    let freeze_path = self.path.join("cgroup.freeze");
    tokio::fs::write(&freeze_path, "1\n")
        .await
        .with_context(|| format!("cgroup: write 1 to {}", freeze_path.display()))?;
    tracing::info!(container_id = %self.id, "cgroup: container paused");
    Ok(())
}

/// Thaw all processes in this container's cgroup.
///
/// Writes `"0"` to `cgroup.freeze`. Requires cgroups v2.
pub async fn resume(&self) -> anyhow::Result<()> {
    let freeze_path = self.path.join("cgroup.freeze");
    tokio::fs::write(&freeze_path, "0\n")
        .await
        .with_context(|| format!("cgroup: write 0 to {}", freeze_path.display()))?;
    tracing::info!(container_id = %self.id, "cgroup: container resumed");
    Ok(())
}
```

Also ensure `CgroupManager` has an `id: String` field (add it if not already present, alongside `path: PathBuf`).

- [ ] **Step 4: Run check**

```bash
cargo check -p minibox
```

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/src/container/cgroups.rs
git commit -m "feat(cgroups): add pause() and resume() via cgroup.freeze"
```

---

### Task 2: Add `ContainerState::Paused` and transition logic

**Files:**

- Modify: `crates/daemonbox/src/state.rs`

- [ ] **Step 1: Write failing test**

In `crates/daemonbox/src/state.rs` inside `#[cfg(test)] mod tests`, add:

```rust
#[tokio::test]
async fn test_pause_resume_state_transitions() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
    let mut state = DaemonState::new(store, tmp.path().to_path_buf()).await.unwrap();

    // Add a running container
    let mut record = make_test_record();
    record.info.state = "running".to_string();
    state.add_container(record.clone()).await.unwrap();
    let id = record.info.id.clone();

    // Pause it
    state.update_container_state(&id, ContainerState::Paused).await.unwrap();
    let c = state.get_container(&id).unwrap();
    assert_eq!(c.info.state, "paused");

    // Resume it
    state.update_container_state(&id, ContainerState::Running).await.unwrap();
    let c = state.get_container(&id).unwrap();
    assert_eq!(c.info.state, "running");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p daemonbox test_pause_resume_state_transitions 2>&1 | tail -5
```

Expected: compile error — `ContainerState::Paused` not found.

- [ ] **Step 3: Add `Paused` to `ContainerState`**

In `crates/daemonbox/src/state.rs`, find the `ContainerState` enum and add:

```rust
/// Container is frozen (cgroup.freeze = 1).
Paused,
```

- [ ] **Step 4: Update `update_container_state` to allow pause/resume transitions**

Find `update_container_state` and add these match arms (alongside existing `Running → Stopped`):

```rust
// Pause: Running → Paused
(ContainerState::Running, ContainerState::Paused) => {
    record.info.state = "paused".to_string();
}
// Resume: Paused → Running
(ContainerState::Paused, ContainerState::Running) => {
    record.info.state = "running".to_string();
}
```

Also update the `ContainerState::from_str` / `Display` impls (if they exist) to handle `"paused"`.

- [ ] **Step 5: Run test**

```bash
cargo test -p daemonbox test_pause_resume_state_transitions 2>&1 | tail -5
```

Expected: test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/daemonbox/src/state.rs
git commit -m "feat(state): add ContainerState::Paused and pause/resume transitions"
```

---

### Task 3: Protocol variants for Pause and Resume

**Files:**

- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/minibox/src/protocol.rs`

- [ ] **Step 1: Add to `DaemonRequest` in `crates/minibox-core/src/protocol.rs`**

After the `Stop` variant, add:

```rust
/// Freeze all processes in a running container via cgroup.freeze.
PauseContainer {
    /// Container ID to pause.
    id: String,
},

/// Thaw a paused container.
ResumeContainer {
    /// Container ID to resume.
    id: String,
},
```

- [ ] **Step 2: Add to `DaemonResponse` in `crates/minibox-core/src/protocol.rs`**

After `Success`, add:

```rust
/// Confirmation that a container was paused.
ContainerPaused {
    /// The container ID.
    id: String,
},

/// Confirmation that a container was resumed.
ContainerResumed {
    /// The container ID.
    id: String,
},
```

- [ ] **Step 3: Mirror changes in `crates/minibox/src/protocol.rs`**

Apply the identical `PauseContainer`, `ResumeContainer`, `ContainerPaused`, `ContainerResumed` additions to `crates/minibox/src/protocol.rs`.

- [ ] **Step 4: Add to `is_terminal_response` in `crates/daemonbox/src/server.rs`**

Add `ContainerPaused` and `ContainerResumed` to the `matches!` block:

```rust
| DaemonResponse::ContainerPaused { .. }
| DaemonResponse::ContainerResumed { .. }
```

- [ ] **Step 5: Check**

```bash
cargo check -p minibox-core -p minibox -p daemonbox
```

Expected: compiles cleanly.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-core/src/protocol.rs crates/minibox/src/protocol.rs crates/daemonbox/src/server.rs
git commit -m "feat(protocol): add PauseContainer/ResumeContainer request+response variants"
```

---

### Task 4: `handle_pause` and `handle_resume` in handler

**Files:**

- Modify: `crates/daemonbox/src/handler.rs`

- [ ] **Step 1: Write failing test**

In `crates/daemonbox/tests/handler_tests.rs`, add:

```rust
#[tokio::test]
async fn test_pause_nonexistent_container_returns_error() {
    let (deps, state) = create_test_deps_with_dir(&tempfile::tempdir().unwrap()).await;
    let req = DaemonRequest::PauseContainer { id: "doesnotexist".to_string() };
    let resp = dispatch_single(&deps, &state, req).await;
    assert!(matches!(resp, DaemonResponse::Error { .. }));
}

#[tokio::test]
async fn test_resume_nonexistent_container_returns_error() {
    let (deps, state) = create_test_deps_with_dir(&tempfile::tempdir().unwrap()).await;
    let req = DaemonRequest::ResumeContainer { id: "doesnotexist".to_string() };
    let resp = dispatch_single(&deps, &state, req).await;
    assert!(matches!(resp, DaemonResponse::Error { .. }));
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p daemonbox test_pause_nonexistent 2>&1 | tail -5
```

Expected: compile error — `handle_pause` not implemented.

- [ ] **Step 3: Add `handle_pause` and `handle_resume` to `handler.rs`**

Add these two functions after `handle_stop`:

```rust
pub(crate) async fn handle_pause(
    id: String,
    state: Arc<Mutex<DaemonState>>,
    tx: Sender<DaemonResponse>,
) {
    let record = {
        let s = state.lock().await;
        s.get_container(&id)
    };
    let record = match record {
        Some(r) => r,
        None => {
            let _ = tx.send(DaemonResponse::Error {
                message: format!("container {id} not found"),
            });
            return;
        }
    };
    if record.info.state != "running" {
        let _ = tx.send(DaemonResponse::Error {
            message: format!("container {id} is not running (state: {})", record.info.state),
        });
        return;
    }
    let cgroup_path = record.cgroup_path.clone();
    let freeze_path = cgroup_path.join("cgroup.freeze");
    if let Err(e) = tokio::fs::write(&freeze_path, "1\n").await {
        let _ = tx.send(DaemonResponse::Error {
            message: format!("pause failed: {e}"),
        });
        return;
    }
    {
        let mut s = state.lock().await;
        if let Err(e) = s.update_container_state(&id, ContainerState::Paused).await {
            tracing::warn!(container_id = %id, error = %e, "state: failed to mark paused");
        }
    }
    tracing::info!(container_id = %id, "container: paused");
    let _ = tx.send(DaemonResponse::ContainerPaused { id });
}

pub(crate) async fn handle_resume(
    id: String,
    state: Arc<Mutex<DaemonState>>,
    tx: Sender<DaemonResponse>,
) {
    let record = {
        let s = state.lock().await;
        s.get_container(&id)
    };
    let record = match record {
        Some(r) => r,
        None => {
            let _ = tx.send(DaemonResponse::Error {
                message: format!("container {id} not found"),
            });
            return;
        }
    };
    if record.info.state != "paused" {
        let _ = tx.send(DaemonResponse::Error {
            message: format!("container {id} is not paused (state: {})", record.info.state),
        });
        return;
    }
    let cgroup_path = record.cgroup_path.clone();
    let freeze_path = cgroup_path.join("cgroup.freeze");
    if let Err(e) = tokio::fs::write(&freeze_path, "0\n").await {
        let _ = tx.send(DaemonResponse::Error {
            message: format!("resume failed: {e}"),
        });
        return;
    }
    {
        let mut s = state.lock().await;
        if let Err(e) = s.update_container_state(&id, ContainerState::Running).await {
            tracing::warn!(container_id = %id, error = %e, "state: failed to mark running");
        }
    }
    tracing::info!(container_id = %id, "container: resumed");
    let _ = tx.send(DaemonResponse::ContainerResumed { id });
}
```

- [ ] **Step 4: Add dispatch arms in `server.rs`**

In `crates/daemonbox/src/server.rs`, find the `dispatch_request` function and add after the `Stop` arm:

```rust
DaemonRequest::PauseContainer { id } => {
    tokio::spawn(handle_pause(id, Arc::clone(&state), tx));
}
DaemonRequest::ResumeContainer { id } => {
    tokio::spawn(handle_resume(id, Arc::clone(&state), tx));
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p daemonbox test_pause_nonexistent test_resume_nonexistent 2>&1 | tail -10
```

Expected: both pass.

- [ ] **Step 6: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/daemonbox/src/server.rs
git commit -m "feat(handler): add handle_pause and handle_resume"
```

---

### Task 5: CLI subcommands — `minibox pause` and `minibox resume`

**Files:**

- Create: `crates/minibox-cli/src/commands/pause.rs`
- Create: `crates/minibox-cli/src/commands/resume.rs`
- Modify: `crates/minibox-cli/src/commands/mod.rs`
- Modify: `crates/minibox-cli/src/main.rs`

- [ ] **Step 1: Create `pause.rs`**

```rust
//! `minibox pause <id>` — freeze a running container.
use anyhow::Result;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

pub async fn run(id: String) -> Result<()> {
    let mut client = DaemonClient::connect().await?;
    let req = DaemonRequest::PauseContainer { id: id.clone() };
    let resp = client.send_and_receive(req).await?;
    match resp {
        DaemonResponse::ContainerPaused { id } => {
            println!("{id}");
            Ok(())
        }
        DaemonResponse::Error { message } => {
            anyhow::bail!("{message}")
        }
        other => anyhow::bail!("unexpected response: {other:?}"),
    }
}
```

- [ ] **Step 2: Create `resume.rs`**

```rust
//! `minibox resume <id>` — thaw a paused container.
use anyhow::Result;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

pub async fn run(id: String) -> Result<()> {
    let mut client = DaemonClient::connect().await?;
    let req = DaemonRequest::ResumeContainer { id: id.clone() };
    let resp = client.send_and_receive(req).await?;
    match resp {
        DaemonResponse::ContainerResumed { id } => {
            println!("{id}");
            Ok(())
        }
        DaemonResponse::Error { message } => {
            anyhow::bail!("{message}")
        }
        other => anyhow::bail!("unexpected response: {other:?}"),
    }
}
```

- [ ] **Step 3: Register in `mod.rs` and `main.rs`**

In `crates/minibox-cli/src/commands/mod.rs`, add:

```rust
pub mod pause;
pub mod resume;
```

In `crates/minibox-cli/src/main.rs`, add the subcommand enum variants and dispatch arms following the existing `stop` pattern. Add `Pause { id: String }` and `Resume { id: String }` to the `Commands` enum, and dispatch to `commands::pause::run(id).await` and `commands::resume::run(id).await`.

- [ ] **Step 4: Check**

```bash
cargo check -p minibox-cli
```

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-cli/src/commands/pause.rs crates/minibox-cli/src/commands/resume.rs \
        crates/minibox-cli/src/commands/mod.rs crates/minibox-cli/src/main.rs
git commit -m "feat(cli): add minibox pause and minibox resume subcommands"
```

---

## Phase 2: Container Events

### Task 6: `EventSink`, `EventSource`, and `BroadcastEventBroker`

**Files:**

- Create: `crates/minibox-core/src/events.rs`
- Modify: `crates/minibox-core/src/lib.rs`
- Modify: `crates/minibox-core/src/domain.rs`

- [ ] **Step 1: Create `crates/minibox-core/src/events.rs`**

```rust
//! Container lifecycle event types and pub/sub ports.
//!
//! `EventSink` is the write port — handlers call `emit()`.
//! `EventSource` is the read port — dashbox and CLI subscribe.
//! `BroadcastEventBroker` is the single adapter implementing both ports.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::broadcast;

/// A structured event emitted by the minibox daemon during container lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContainerEvent {
    Created   { id: String, image: String,      timestamp: SystemTime },
    Started   { id: String, pid: u32,           timestamp: SystemTime },
    Stopped   { id: String, exit_code: i32,     timestamp: SystemTime },
    Paused    { id: String,                     timestamp: SystemTime },
    Resumed   { id: String,                     timestamp: SystemTime },
    OomKilled { id: String,                     timestamp: SystemTime },
    ImagePulled  { image: String, size_bytes: u64, timestamp: SystemTime },
    ImageRemoved { image: String,                  timestamp: SystemTime },
    ImagePruned  { count: usize, freed_bytes: u64, timestamp: SystemTime },
}

/// Port: write-only event emission. Handlers depend on this.
pub trait EventSink: Send + Sync {
    /// Emit an event. Fire-and-forget — never blocks.
    fn emit(&self, event: ContainerEvent);
}

/// Port: subscribe to the event stream. Dashbox and CLI depend on this.
pub trait EventSource: Send + Sync {
    /// Returns a receiver that will receive all future events.
    /// Lagged receivers (too slow to consume) receive `RecvError::Lagged`.
    fn subscribe(&self) -> broadcast::Receiver<ContainerEvent>;
}

/// Adapter: tokio broadcast channel. Implements both `EventSink` and `EventSource`.
///
/// Capacity: 1024 events. Slow consumers receive `RecvError::Lagged` and skip
/// missed events — this is intentional (events are best-effort observability).
#[derive(Clone)]
pub struct BroadcastEventBroker {
    tx: broadcast::Sender<ContainerEvent>,
}

impl BroadcastEventBroker {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }
}

impl Default for BroadcastEventBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for BroadcastEventBroker {
    fn emit(&self, event: ContainerEvent) {
        // send() errors only if there are no receivers — that's fine.
        let _ = self.tx.send(event);
    }
}

impl EventSource for BroadcastEventBroker {
    fn subscribe(&self) -> broadcast::Receiver<ContainerEvent> {
        self.tx.subscribe()
    }
}

/// No-op sink for tests and platforms where events are not needed.
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: ContainerEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_emit_and_receive() {
        let broker = BroadcastEventBroker::new();
        let mut rx = broker.subscribe();

        broker.emit(ContainerEvent::Created {
            id: "abc".to_string(),
            image: "alpine".to_string(),
            timestamp: SystemTime::now(),
        });

        let evt = rx.recv().await.unwrap();
        assert!(matches!(evt, ContainerEvent::Created { id, .. } if id == "abc"));
    }

    #[test]
    fn test_noop_sink_does_not_panic() {
        let sink = NoopEventSink;
        sink.emit(ContainerEvent::Stopped {
            id: "x".to_string(),
            exit_code: 0,
            timestamp: SystemTime::now(),
        });
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let broker = BroadcastEventBroker::new();
        let mut rx1 = broker.subscribe();
        let mut rx2 = broker.subscribe();

        broker.emit(ContainerEvent::Paused {
            id: "c1".to_string(),
            timestamp: SystemTime::now(),
        });

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert!(matches!(e1, ContainerEvent::Paused { .. }));
        assert!(matches!(e2, ContainerEvent::Paused { .. }));
    }
}
```

- [ ] **Step 2: Register module in `lib.rs`**

In `crates/minibox-core/src/lib.rs`, add:

```rust
pub mod events;
```

- [ ] **Step 3: Add type aliases to `domain.rs`**

In `crates/minibox-core/src/domain.rs`, after the existing `DynNetworkProvider` line, add:

```rust
/// Type alias for a shared, dynamic [`EventSink`] implementation.
pub type DynEventSink = Arc<dyn crate::events::EventSink>;
/// Type alias for a shared, dynamic [`EventSource`] implementation.
pub type DynEventSource = Arc<dyn crate::events::EventSource>;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minibox-core events 2>&1 | tail -10
```

Expected: all 3 event tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-core/src/events.rs crates/minibox-core/src/lib.rs crates/minibox-core/src/domain.rs
git commit -m "feat(events): add EventSink/EventSource ports and BroadcastEventBroker adapter"
```

---

### Task 7: Inject `event_sink` into `HandlerDependencies` and emit events

**Files:**

- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/miniboxd/src/main.rs`

- [ ] **Step 1: Add `event_sink` to `HandlerDependencies`**

In `crates/daemonbox/src/handler.rs`, add to the struct:

```rust
pub struct HandlerDependencies {
    // ... existing fields ...
    pub event_sink: Arc<dyn minibox_core::events::EventSink>,
}
```

Update `HandlerDependencies::new(...)` (or wherever it is constructed) to accept and store the `event_sink`.

- [ ] **Step 2: Emit `Created` and `Started` events in `handle_run`**

Find the point in `handle_run` (or `handle_run_streaming`) where `ContainerCreated` is sent. Immediately before, add:

```rust
deps.event_sink.emit(ContainerEvent::Created {
    id: container_id.to_string(),
    image: format!("{}:{}", req.image, req.tag),
    timestamp: std::time::SystemTime::now(),
});
```

Find where the container PID is known (after `spawn_blocking` returns). Add:

```rust
deps.event_sink.emit(ContainerEvent::Started {
    id: container_id.to_string(),
    pid: pid as u32,
    timestamp: std::time::SystemTime::now(),
});
```

- [ ] **Step 3: Emit `Stopped` or `OomKilled` in `daemon_wait_for_exit`**

Find `daemon_wait_for_exit` (or equivalent reaper). After setting container state to `Stopped`, add:

```rust
// Check if OOM-killed by reading memory.oom_control or memory.events
let oom = check_oom_killed(&cgroup_path).await;
if oom {
    event_sink.emit(ContainerEvent::OomKilled {
        id: container_id.clone(),
        timestamp: std::time::SystemTime::now(),
    });
} else {
    event_sink.emit(ContainerEvent::Stopped {
        id: container_id.clone(),
        exit_code,
        timestamp: std::time::SystemTime::now(),
    });
}
```

Add a helper (can be simple for now):

```rust
async fn check_oom_killed(cgroup_path: &Path) -> bool {
    // cgroup v2: memory.events contains "oom_kill N"
    let events_path = cgroup_path.join("memory.events");
    if let Ok(content) = tokio::fs::read_to_string(&events_path).await {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("oom_kill ") {
                return rest.trim().parse::<u64>().unwrap_or(0) > 0;
            }
        }
    }
    false
}
```

- [ ] **Step 4: Emit `Paused`/`Resumed` in `handle_pause`/`handle_resume`**

In `handle_pause`, after the successful `tx.send(ContainerPaused)`, add:

```rust
deps.event_sink.emit(ContainerEvent::Paused {
    id: id.clone(),
    timestamp: std::time::SystemTime::now(),
});
```

Same pattern for `handle_resume` with `ContainerEvent::Resumed`.

- [ ] **Step 5: Wire `BroadcastEventBroker` in `miniboxd/src/main.rs`**

```rust
use minibox_core::events::BroadcastEventBroker;

let event_broker = Arc::new(BroadcastEventBroker::new());
// Pass Arc::clone(&event_broker) as event_sink to HandlerDependencies
// Store Arc::clone(&event_broker) as event_source for SubscribeEvents handler (Task 8)
```

- [ ] **Step 6: Check**

```bash
cargo check -p daemonbox -p miniboxd
```

Expected: compiles cleanly.

- [ ] **Step 7: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/miniboxd/src/main.rs
git commit -m "feat(handler): inject EventSink and emit lifecycle events"
```

---

### Task 8: Protocol + handler for `SubscribeEvents`, CLI `minibox events`

**Files:**

- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/minibox/src/protocol.rs`
- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`
- Create: `crates/minibox-cli/src/commands/events.rs`
- Modify: `crates/minibox-cli/src/commands/mod.rs`
- Modify: `crates/minibox-cli/src/main.rs`

- [ ] **Step 1: Add protocol variants**

In `crates/minibox-core/src/protocol.rs` `DaemonRequest`:

```rust
/// Subscribe to the container event stream.
/// Daemon will send `Event` responses until the connection closes.
SubscribeEvents,
```

In `DaemonResponse`:

```rust
/// A container lifecycle event.
///
/// Non-terminal: sent zero or more times until the connection closes.
Event {
    /// The serialized event payload.
    event: minibox_core::events::ContainerEvent,
},
```

Mirror both in `crates/minibox/src/protocol.rs`.

- [ ] **Step 2: `Event` is non-terminal — update `is_terminal_response`**

In `crates/daemonbox/src/server.rs`, the `Event` variant must NOT appear in `is_terminal_response`. Verify it isn't accidentally added. The function only lists terminal variants — `Event` is streaming like `ContainerOutput`.

- [ ] **Step 3: Add `handle_subscribe_events` to `handler.rs`**

```rust
pub(crate) async fn handle_subscribe_events(
    event_source: Arc<dyn minibox_core::events::EventSource>,
    tx: Sender<DaemonResponse>,
) {
    let mut rx = event_source.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if tx.send(DaemonResponse::Event { event }).is_err() {
                    // Client disconnected
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(skipped = n, "events: subscriber lagged, skipping events");
                // Continue — don't break on lag
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}
```

- [ ] **Step 4: Dispatch arm in `server.rs`**

```rust
DaemonRequest::SubscribeEvents => {
    tokio::spawn(handle_subscribe_events(Arc::clone(&event_source), tx));
}
```

(The `event_source: Arc<dyn EventSource>` must be passed through to the dispatch function — add it to the function signature alongside `state` and `deps`.)

- [ ] **Step 5: Create `events.rs` CLI command**

```rust
//! `minibox events` — stream container events as JSON-lines to stdout.
use anyhow::Result;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

pub async fn run() -> Result<()> {
    let mut client = DaemonClient::connect().await?;
    client.send(DaemonRequest::SubscribeEvents).await?;

    loop {
        match client.receive().await? {
            DaemonResponse::Event { event } => {
                let line = serde_json::to_string(&event)?;
                println!("{line}");
            }
            DaemonResponse::Error { message } => {
                anyhow::bail!("{message}");
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 6: Register in `mod.rs` and `main.rs`**

Add `pub mod events;` to `commands/mod.rs`. Add `Events` (no args) to the CLI `Commands` enum and dispatch to `commands::events::run().await`.

- [ ] **Step 7: Check**

```bash
cargo check -p minibox-core -p minibox -p daemonbox -p minibox-cli
```

Expected: compiles cleanly.

- [ ] **Step 8: Commit**

```bash
git add crates/minibox-core/src/protocol.rs crates/minibox/src/protocol.rs \
        crates/daemonbox/src/handler.rs crates/daemonbox/src/server.rs \
        crates/minibox-cli/src/commands/events.rs \
        crates/minibox-cli/src/commands/mod.rs crates/minibox-cli/src/main.rs
git commit -m "feat(events): add SubscribeEvents protocol + minibox events CLI command"
```

---

## Phase 3: Image GC + Leases

### Task 9: `list_all_images()`, `delete_image()`, `image_size_bytes()` on `ImageStore`

**Files:**

- Modify: `crates/minibox-core/src/image/mod.rs`

- [ ] **Step 1: Write failing tests**

In `crates/minibox-core/src/image/mod.rs` test module, add:

```rust
#[tokio::test]
async fn test_list_all_images_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ImageStore::new(tmp.path()).unwrap();
    let images = store.list_all_images().await.unwrap();
    assert!(images.is_empty());
}

#[tokio::test]
async fn test_delete_image_removes_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ImageStore::new(tmp.path()).unwrap();
    // Seed a fake image dir
    let img_dir = tmp.path().join("alpine").join("latest");
    tokio::fs::create_dir_all(&img_dir).await.unwrap();
    tokio::fs::write(img_dir.join("manifest.json"), b"{}").await.unwrap();

    store.delete_image("alpine", "latest").await.unwrap();

    assert!(!img_dir.exists());
}
```

- [ ] **Step 2: Run to verify failures**

```bash
cargo test -p minibox-core test_list_all_images test_delete_image 2>&1 | tail -5
```

Expected: compile error — methods not found.

- [ ] **Step 3: Add methods to `ImageStore`**

```rust
/// List all `"name:tag"` strings known to this store.
///
/// Walks `{base_dir}/*/` directories looking for `manifest.json`.
pub async fn list_all_images(&self) -> anyhow::Result<Vec<String>> {
    let mut result = Vec::new();
    let mut rd = tokio::fs::read_dir(&self.base_dir).await?;
    while let Some(name_entry) = rd.next_entry().await? {
        if !name_entry.file_type().await?.is_dir() { continue; }
        let name = name_entry.file_name().to_string_lossy().replace('_', "/");
        let mut td = tokio::fs::read_dir(name_entry.path()).await?;
        while let Some(tag_entry) = td.next_entry().await? {
            if !tag_entry.file_type().await?.is_dir() { continue; }
            let manifest = tag_entry.path().join("manifest.json");
            if manifest.exists() {
                let tag = tag_entry.file_name().to_string_lossy().to_string();
                result.push(format!("{name}:{tag}"));
            }
        }
    }
    Ok(result)
}

/// Return the total disk usage of an image's layer dirs in bytes.
pub async fn image_size_bytes(&self, name: &str, tag: &str) -> anyhow::Result<u64> {
    let dir = self.image_dir(name, tag)?;
    let mut total = 0u64;
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&d).await?;
        while let Some(e) = rd.next_entry().await? {
            let meta = e.metadata().await?;
            if meta.is_dir() {
                stack.push(e.path());
            } else {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

/// Delete an image's manifest and all layer directories.
///
/// Best-effort: logs a warning if the directory cannot be removed.
pub async fn delete_image(&self, name: &str, tag: &str) -> anyhow::Result<()> {
    let dir = self.image_dir(name, tag)?;
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .with_context(|| format!("image: remove_dir_all {}", dir.display()))?;
        tracing::info!(image = %format!("{name}:{tag}"), "image: deleted");
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minibox-core test_list_all_images test_delete_image 2>&1 | tail -10
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-core/src/image/mod.rs
git commit -m "feat(image): add list_all_images, delete_image, image_size_bytes to ImageStore"
```

---

### Task 10: `DiskLeaseService` and `ImageLeaseService` trait

**Files:**

- Create: `crates/minibox-core/src/image/lease.rs`
- Modify: `crates/minibox-core/src/image/mod.rs`

- [ ] **Step 1: Write failing test**

Create a temp file `crates/minibox-core/src/image/lease.rs` with just:

```rust
// placeholder — will be replaced
```

Add to `crates/minibox-core/src/image/mod.rs`:

```rust
pub mod lease;
```

Then add this test to `lease.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_acquire_and_release() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = DiskLeaseService::new(tmp.path().join("leases.json")).await.unwrap();

        let lease_id = svc.acquire("alpine:latest", Duration::from_secs(3600)).await.unwrap();
        let leases = svc.list().await.unwrap();
        assert_eq!(leases.len(), 1);
        assert!(leases[0].image_refs.contains("alpine:latest"));

        svc.release(&lease_id).await.unwrap();
        let leases = svc.list().await.unwrap();
        assert!(leases.is_empty());
    }

    #[tokio::test]
    async fn test_expired_lease_not_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = DiskLeaseService::new(tmp.path().join("leases.json")).await.unwrap();

        // Acquire with 0-second TTL (immediately expired)
        let _id = svc.acquire("old:image", Duration::from_secs(0)).await.unwrap();
        let active = svc.list_active().await.unwrap();
        assert!(active.is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failures**

```bash
cargo test -p minibox-core lease 2>&1 | tail -5
```

Expected: compile error — `DiskLeaseService` not defined.

- [ ] **Step 3: Implement `lease.rs`**

```rust
//! Lease service: protect images from GC during in-flight operations.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use uuid::Uuid;

/// A lease protecting one or more image refs from garbage collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub id:         String,
    pub created_at: SystemTime,
    pub expire_at:  SystemTime,
    /// Image `"name:tag"` strings protected by this lease.
    pub image_refs: HashSet<String>,
}

/// Port: lease lifecycle management.
#[async_trait]
pub trait ImageLeaseService: Send + Sync {
    /// Protect `image_ref` from GC for `ttl`. Returns the new lease ID.
    async fn acquire(&self, image_ref: &str, ttl: Duration) -> Result<String>;
    /// Release a lease early (image can now be GC'd if not otherwise protected).
    async fn release(&self, lease_id: &str) -> Result<()>;
    /// Extend a lease's expiry by an additional `ttl`.
    async fn extend(&self, lease_id: &str, ttl: Duration) -> Result<()>;
    /// All leases (including expired).
    async fn list(&self) -> Result<Vec<LeaseRecord>>;
    /// Only non-expired leases.
    async fn list_active(&self) -> Result<Vec<LeaseRecord>>;
    /// Returns true if any active lease covers `image_ref`.
    async fn is_leased(&self, image_ref: &str) -> Result<bool>;
}

/// Disk-backed lease service. Persists to a single JSON file.
pub struct DiskLeaseService {
    leases: Arc<RwLock<HashMap<String, LeaseRecord>>>,
    path:   PathBuf,
}

impl DiskLeaseService {
    pub async fn new(path: PathBuf) -> Result<Self> {
        let leases = if path.exists() {
            let bytes = tokio::fs::read(&path).await
                .with_context(|| format!("lease: read {}", path.display()))?;
            serde_json::from_slice(&bytes)
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self {
            leases: Arc::new(RwLock::new(leases)),
            path,
        })
    }

    async fn persist(&self) -> Result<()> {
        let leases = self.leases.read().await;
        let bytes = serde_json::to_vec_pretty(&*leases)?;
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

#[async_trait]
impl ImageLeaseService for DiskLeaseService {
    async fn acquire(&self, image_ref: &str, ttl: Duration) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = SystemTime::now();
        let record = LeaseRecord {
            id: id.clone(),
            created_at: now,
            expire_at: now + ttl,
            image_refs: std::iter::once(image_ref.to_string()).collect(),
        };
        self.leases.write().await.insert(id.clone(), record);
        self.persist().await?;
        Ok(id)
    }

    async fn release(&self, lease_id: &str) -> Result<()> {
        self.leases.write().await.remove(lease_id);
        self.persist().await
    }

    async fn extend(&self, lease_id: &str, ttl: Duration) -> Result<()> {
        let mut leases = self.leases.write().await;
        if let Some(l) = leases.get_mut(lease_id) {
            l.expire_at = SystemTime::now() + ttl;
            drop(leases);
            self.persist().await?;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<LeaseRecord>> {
        Ok(self.leases.read().await.values().cloned().collect())
    }

    async fn list_active(&self) -> Result<Vec<LeaseRecord>> {
        let now = SystemTime::now();
        Ok(self.leases.read().await.values()
            .filter(|l| l.expire_at > now)
            .cloned()
            .collect())
    }

    async fn is_leased(&self, image_ref: &str) -> Result<bool> {
        let now = SystemTime::now();
        Ok(self.leases.read().await.values().any(|l| {
            l.expire_at > now && l.image_refs.contains(image_ref)
        }))
    }
}
```

Add `uuid` to `minibox-core/Cargo.toml`:

```toml
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minibox-core lease 2>&1 | tail -10
```

Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-core/src/image/lease.rs crates/minibox-core/src/image/mod.rs \
        crates/minibox-core/Cargo.toml
git commit -m "feat(image): add DiskLeaseService and ImageLeaseService trait"
```

---

### Task 11: `ImageGc` and `prune` operation

**Files:**

- Create: `crates/minibox-core/src/image/gc.rs`
- Modify: `crates/minibox-core/src/image/mod.rs`

- [ ] **Step 1: Write failing test**

Create `gc.rs` with placeholder, add `pub mod gc;` to `image/mod.rs`. Add tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::lease::DiskLeaseService;
    use std::sync::Arc;

    async fn make_gc(tmp: &tempfile::TempDir) -> ImageGc {
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
        let leases = Arc::new(
            DiskLeaseService::new(tmp.path().join("leases.json")).await.unwrap()
        );
        ImageGc::new(store, leases)
    }

    #[tokio::test]
    async fn test_prune_empty_store_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let gc = make_gc(&tmp).await;
        let report = gc.prune(false, &[]).await.unwrap();
        assert_eq!(report.removed.len(), 0);
        assert_eq!(report.freed_bytes, 0);
    }

    #[tokio::test]
    async fn test_prune_removes_unreferenced_image() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());

        // Seed a fake image
        let img_dir = tmp.path().join("images").join("alpine").join("latest");
        tokio::fs::create_dir_all(&img_dir).await.unwrap();
        tokio::fs::write(img_dir.join("manifest.json"), b"{}").await.unwrap();

        let leases = Arc::new(
            DiskLeaseService::new(tmp.path().join("leases.json")).await.unwrap()
        );
        let gc = ImageGc::new(Arc::clone(&store), leases);

        // in_use: empty (no containers using alpine:latest)
        let report = gc.prune(false, &[]).await.unwrap();
        assert_eq!(report.removed, vec!["alpine/latest:latest"]);
    }

    #[tokio::test]
    async fn test_prune_dry_run_does_not_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());

        let img_dir = tmp.path().join("images").join("alpine").join("latest");
        tokio::fs::create_dir_all(&img_dir).await.unwrap();
        tokio::fs::write(img_dir.join("manifest.json"), b"{}").await.unwrap();

        let leases = Arc::new(
            DiskLeaseService::new(tmp.path().join("leases.json")).await.unwrap()
        );
        let gc = ImageGc::new(Arc::clone(&store), leases);
        let report = gc.prune(true, &[]).await.unwrap();

        assert!(report.dry_run);
        assert!(!report.removed.is_empty());
        // Directory must still exist
        assert!(img_dir.exists());
    }
}
```

- [ ] **Step 2: Implement `gc.rs`**

```rust
//! Image garbage collection.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;

use super::lease::ImageLeaseService;
use super::ImageStore;

/// Summary of a prune operation.
#[derive(Debug, Default)]
pub struct PruneReport {
    /// Image refs that were (or would be) removed.
    pub removed: Vec<String>,
    /// Bytes freed (or that would be freed in dry-run mode).
    pub freed_bytes: u64,
    /// True if this was a dry run (no files actually deleted).
    pub dry_run: bool,
}

/// Port: remove unused images.
#[async_trait]
pub trait ImageGarbageCollector: Send + Sync {
    /// Remove images not referenced by active containers or valid leases.
    ///
    /// `in_use` is a slice of `"name:tag"` strings for images currently used
    /// by running or paused containers.
    async fn prune(&self, dry_run: bool, in_use: &[String]) -> Result<PruneReport>;
}

/// Adapter: GC implementation using `ImageStore` + `ImageLeaseService`.
pub struct ImageGc {
    store:  Arc<ImageStore>,
    leases: Arc<dyn ImageLeaseService>,
}

impl ImageGc {
    pub fn new(store: Arc<ImageStore>, leases: Arc<dyn ImageLeaseService>) -> Self {
        Self { store, leases }
    }
}

#[async_trait]
impl ImageGarbageCollector for ImageGc {
    async fn prune(&self, dry_run: bool, in_use: &[String]) -> Result<PruneReport> {
        let all = self.store.list_all_images().await?;
        let in_use_set: HashSet<&str> = in_use.iter().map(|s| s.as_str()).collect();

        let mut report = PruneReport { dry_run, ..Default::default() };

        for image_ref in &all {
            // Skip images in use by running/paused containers
            if in_use_set.contains(image_ref.as_str()) {
                continue;
            }
            // Skip images protected by an active lease
            if self.leases.is_leased(image_ref).await? {
                continue;
            }

            // Parse "name:tag"
            let (name, tag) = match image_ref.rsplit_once(':') {
                Some(pair) => pair,
                None => continue,
            };

            let size = self.store.image_size_bytes(name, tag).await.unwrap_or(0);
            report.freed_bytes += size;
            report.removed.push(image_ref.clone());

            if !dry_run {
                if let Err(e) = self.store.delete_image(name, tag).await {
                    tracing::warn!(image = %image_ref, error = %e, "gc: failed to delete image");
                }
            }
        }

        Ok(report)
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p minibox-core gc 2>&1 | tail -10
```

Expected: all 3 GC tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-core/src/image/gc.rs crates/minibox-core/src/image/mod.rs
git commit -m "feat(image): add ImageGc and ImageGarbageCollector trait"
```

---

### Task 12: Protocol + handler for `Prune` and `Rmi`; CLI commands

**Files:**

- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/minibox/src/protocol.rs`
- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`
- Create: `crates/minibox-cli/src/commands/prune.rs`
- Create: `crates/minibox-cli/src/commands/rmi.rs`
- Modify: `crates/minibox-cli/src/commands/mod.rs`
- Modify: `crates/minibox-cli/src/main.rs`

- [ ] **Step 1: Add protocol variants**

In `DaemonRequest`:

```rust
/// Remove unused images (optionally dry-run).
Prune {
    #[serde(default)]
    dry_run: bool,
},
/// Remove a specific image by reference.
RemoveImage {
    /// Image reference, e.g. `"alpine:latest"`.
    image_ref: String,
},
```

In `DaemonResponse` (add to `is_terminal_response`):

```rust
/// Result of a prune operation.
Pruned {
    removed: Vec<String>,
    freed_bytes: u64,
    dry_run: bool,
},
```

Mirror in `crates/minibox/src/protocol.rs`. Add `Pruned` to `is_terminal_response`.

- [ ] **Step 2: Add `image_gc` field to `HandlerDependencies`**

```rust
pub image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector>,
```

- [ ] **Step 3: Add `handle_prune` to `handler.rs`**

```rust
pub(crate) async fn handle_prune(
    dry_run: bool,
    state: Arc<Mutex<DaemonState>>,
    image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector>,
    event_sink: Arc<dyn minibox_core::events::EventSink>,
    tx: Sender<DaemonResponse>,
) {
    // Collect in-use image refs from running/paused containers
    let in_use: Vec<String> = {
        let s = state.lock().await;
        s.list_containers()
            .into_iter()
            .filter_map(|c| {
                if c.state == "running" || c.state == "paused" {
                    Some(c.image.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    match image_gc.prune(dry_run, &in_use).await {
        Ok(report) => {
            let count = report.removed.len();
            let freed = report.freed_bytes;
            event_sink.emit(minibox_core::events::ContainerEvent::ImagePruned {
                count,
                freed_bytes: freed,
                timestamp: std::time::SystemTime::now(),
            });
            let _ = tx.send(DaemonResponse::Pruned {
                removed: report.removed,
                freed_bytes: freed,
                dry_run: report.dry_run,
            });
        }
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: e.to_string() });
        }
    }
}
```

Add `handle_remove_image` following a similar pattern: validate image is not in use, call `store.delete_image`, emit `ImageRemoved` event, respond `Success`.

- [ ] **Step 4: Dispatch arms in `server.rs`**

```rust
DaemonRequest::Prune { dry_run } => {
    tokio::spawn(handle_prune(dry_run, Arc::clone(&state), Arc::clone(&deps.image_gc), Arc::clone(&deps.event_sink), tx));
}
DaemonRequest::RemoveImage { image_ref } => {
    tokio::spawn(handle_remove_image(image_ref, Arc::clone(&state), Arc::clone(&deps), tx));
}
```

- [ ] **Step 5: Create `prune.rs`**

```rust
//! `minibox prune [--dry-run]` — remove unused images.
use anyhow::Result;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

pub async fn run(dry_run: bool) -> Result<()> {
    let mut client = DaemonClient::connect().await?;
    let resp = client.send_and_receive(DaemonRequest::Prune { dry_run }).await?;
    match resp {
        DaemonResponse::Pruned { removed, freed_bytes, dry_run } => {
            let prefix = if dry_run { "[dry-run] " } else { "" };
            for r in &removed {
                println!("{prefix}Deleted: {r}");
            }
            let freed_mb = freed_bytes as f64 / 1_048_576.0;
            println!("{prefix}Total freed: {freed_mb:.1} MB ({} image{})",
                removed.len(), if removed.len() == 1 { "" } else { "s" });
            Ok(())
        }
        DaemonResponse::Error { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected response: {other:?}"),
    }
}
```

- [ ] **Step 6: Create `rmi.rs`**

```rust
//! `minibox rmi <image>` — remove a specific image.
use anyhow::Result;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

pub async fn run(image_ref: String) -> Result<()> {
    let mut client = DaemonClient::connect().await?;
    let resp = client.send_and_receive(DaemonRequest::RemoveImage { image_ref }).await?;
    match resp {
        DaemonResponse::Success { message } => {
            println!("{message}");
            Ok(())
        }
        DaemonResponse::Error { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected response: {other:?}"),
    }
}
```

- [ ] **Step 7: Register commands**

Add `pub mod prune; pub mod rmi;` to `commands/mod.rs`. Add `Prune { #[arg(long)] dry_run: bool }` and `Rmi { image_ref: String }` to the CLI enum. Dispatch accordingly.

- [ ] **Step 8: Check**

```bash
cargo check -p minibox-core -p minibox -p daemonbox -p minibox-cli
```

Expected: compiles cleanly.

- [ ] **Step 9: Run all unit tests**

```bash
cargo xtask test-unit
```

Expected: all pass.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat(gc): add Prune/RemoveImage protocol, handler, and minibox prune/rmi CLI commands"
```

---

## Phase 4: Bridge Networking

### Task 13: `IpAllocator` and IP management

**Files:**

- Create: `crates/minibox/src/adapters/network/bridge.rs` (initial skeleton)

- [ ] **Step 1: Write failing test**

Create `crates/minibox/src/adapters/network/bridge.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn test_ip_allocator_skips_network_and_gateway() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let first = alloc.allocate().unwrap();
        // Must not be .0 (network) or .1 (gateway)
        assert_ne!(first, "172.20.0.0".parse::<IpAddr>().unwrap());
        assert_ne!(first, "172.20.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(first, "172.20.0.2".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_ip_allocator_release_and_reuse() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let ip1 = alloc.allocate().unwrap();
        alloc.release(ip1);
        let ip2 = alloc.allocate().unwrap();
        assert_eq!(ip1, ip2); // released IP is reused
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p minibox bridge 2>&1 | tail -5
```

Expected: compile error — `IpAllocator` not found.

- [ ] **Step 3: Implement `IpAllocator`**

```rust
//! Bridge network adapter — Linux-only.
#![cfg(target_os = "linux")]

use ipnet::IpNet;
use std::collections::BTreeSet;
use std::net::IpAddr;

/// Sequential IP allocator within a subnet.
///
/// Skips the network address (`.0`) and gateway address (`.1`).
/// Released IPs are returned to the pool.
pub struct IpAllocator {
    subnet:    IpNet,
    available: BTreeSet<u32>,  // IPv4 host parts only
    gateway:   u32,
}

impl IpAllocator {
    pub fn new(subnet: IpNet) -> Self {
        let base = match subnet.network() {
            IpAddr::V4(a) => u32::from(a),
            IpAddr::V6(_) => panic!("IPv6 not supported in IpAllocator"),
        };
        let hosts = subnet.hosts().filter_map(|ip| {
            if let IpAddr::V4(a) = ip { Some(u32::from(a)) } else { None }
        });
        let mut available: BTreeSet<u32> = hosts.collect();
        let gateway = base + 1;
        available.remove(&gateway);  // reserve gateway
        Self { subnet, available, gateway }
    }

    pub fn allocate(&mut self) -> Option<IpAddr> {
        self.available.pop_first().map(|n| IpAddr::V4(n.into()))
    }

    pub fn release(&mut self, ip: IpAddr) {
        if let IpAddr::V4(a) = ip {
            let n = u32::from(a);
            if self.subnet.contains(&ip) && n != self.gateway {
                self.available.insert(n);
            }
        }
    }

    pub fn gateway(&self) -> IpAddr {
        IpAddr::V4(self.gateway.into())
    }
}
```

Add `ipnet` to `crates/minibox/Cargo.toml`:

```toml
ipnet = "2"
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minibox bridge::tests 2>&1 | tail -10
```

Expected: both allocator tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/src/adapters/network/bridge.rs crates/minibox/Cargo.toml
git commit -m "feat(network): add IpAllocator for bridge subnet management"
```

---

### Task 14: `BridgeNetwork` adapter — `setup()` and `attach()`

**Files:**

- Modify: `crates/minibox/src/adapters/network/bridge.rs`
- Modify: `crates/minibox/src/adapters/network/mod.rs`

- [ ] **Step 1: Implement `BridgeNetwork` struct and `setup()`**

Add to `bridge.rs` (Linux-only, after `IpAllocator`):

```rust
use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::domain::{NetworkConfig, NetworkProvider, NetworkStats};
use minibox_core::as_any;
use std::net::IpAddr;
use std::process::Command;
use std::sync::{Arc, Mutex};

const DEFAULT_BRIDGE:  &str = "minibox0";
const DEFAULT_SUBNET:  &str = "172.20.0.0/16";

pub struct BridgeNetwork {
    bridge_name: String,
    subnet:      IpNet,
    ip_alloc:    Arc<Mutex<IpAllocator>>,
    dns_servers: Vec<IpAddr>,
}

impl BridgeNetwork {
    pub fn new() -> Result<Self> {
        let subnet: IpNet = DEFAULT_SUBNET.parse()?;
        Ok(Self {
            bridge_name:  DEFAULT_BRIDGE.to_string(),
            subnet:       subnet.clone(),
            ip_alloc:     Arc::new(Mutex::new(IpAllocator::new(subnet))),
            dns_servers:  vec!["8.8.8.8".parse()?, "1.1.1.1".parse()?],
        })
    }

    /// Ensure bridge interface exists and is up with the gateway IP.
    fn ensure_bridge(&self) -> Result<()> {
        // Create bridge if it doesn't exist
        let exists = Command::new("ip")
            .args(["link", "show", &self.bridge_name])
            .output()?.status.success();

        if !exists {
            run_cmd(&["ip", "link", "add", &self.bridge_name, "type", "bridge"])?;
            let gw = self.ip_alloc.lock().unwrap().gateway().to_string();
            let gw_cidr = format!("{}/{}", gw, self.subnet.prefix_len());
            run_cmd(&["ip", "addr", "add", &gw_cidr, "dev", &self.bridge_name])?;
            run_cmd(&["ip", "link", "set", &self.bridge_name, "up"])?;
        }
        Ok(())
    }

    /// Enable IP forwarding and add MASQUERADE rule if not present.
    fn ensure_nat(&self) -> Result<()> {
        std::fs::write("/proc/sys/net/ipv4/ip_forward", "1")
            .context("enable ip_forward")?;
        // Add MASQUERADE rule (idempotent — iptables ignores duplicates with -C)
        let subnet = self.subnet.to_string();
        let check = Command::new("iptables")
            .args(["-t", "nat", "-C", "POSTROUTING",
                   "-s", &subnet, "-j", "MASQUERADE"])
            .status()?;
        if !check.success() {
            run_cmd(&["iptables", "-t", "nat", "-A", "POSTROUTING",
                      "-s", &subnet, "-j", "MASQUERADE"])?;
        }
        Ok(())
    }
}

fn run_cmd(args: &[&str]) -> Result<()> {
    let status = Command::new(args[0]).args(&args[1..]).status()
        .with_context(|| format!("spawn {}", args[0]))?;
    if !status.success() {
        anyhow::bail!("command failed: {}", args.join(" "));
    }
    Ok(())
}

as_any!(BridgeNetwork);

#[async_trait]
impl NetworkProvider for BridgeNetwork {
    async fn setup(
        &self,
        container_id: &str,
        _config: &NetworkConfig,
    ) -> Result<String> {
        self.ensure_bridge()
            .context("network: ensure bridge")?;
        self.ensure_nat()
            .context("network: ensure nat")?;

        let container_ip = {
            let mut alloc = self.ip_alloc.lock().unwrap();
            alloc.allocate()
                .ok_or_else(|| anyhow::anyhow!("network: IP pool exhausted"))?
        };

        // Create veth pair: host side = veth-{prefix}, container side = ceth-{prefix}
        let prefix = &container_id[..8.min(container_id.len())];
        let host_veth = format!("veth-{prefix}");
        let ceth = format!("ceth-{prefix}");

        run_cmd(&["ip", "link", "add", &host_veth, "type", "veth",
                  "peer", "name", &ceth])?;
        run_cmd(&["ip", "link", "set", &host_veth, "master", &self.bridge_name])?;
        run_cmd(&["ip", "link", "set", &host_veth, "up"])?;

        tracing::info!(
            container_id = %container_id,
            container_ip = %container_ip,
            host_veth = %host_veth,
            "network: bridge setup complete"
        );

        // Return JSON blob with context needed for attach()
        Ok(serde_json::to_string(&serde_json::json!({
            "container_ip": container_ip.to_string(),
            "ceth": ceth,
            "gateway": self.ip_alloc.lock().unwrap().gateway().to_string(),
            "dns": self.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        }))?)
    }

    async fn attach(&self, container_id: &str, pid: u32) -> Result<()> {
        // The setup() return value (JSON) is not available here — store it in a
        // side-channel (a temp file keyed by container_id).
        // Simple approach: read from /run/minibox/net/{container_id}.json
        let net_file = format!("/run/minibox/net/{container_id}.json");
        let ctx: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&net_file)
                .with_context(|| format!("network: read {net_file}"))?
        )?;

        let ceth = ctx["ceth"].as_str().unwrap_or("eth0");
        let container_ip = ctx["container_ip"].as_str().unwrap_or("");
        let gateway = ctx["gateway"].as_str().unwrap_or("");
        let dns: Vec<String> = ctx["dns"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let prefix = &container_id[..8.min(container_id.len())];
        let gw_cidr = format!("{}/{}", container_ip, self.subnet.prefix_len());

        // Move container-side veth into the container's network namespace
        run_cmd(&["ip", "link", "set", ceth, "netns", &pid.to_string()])?;

        // Configure interface inside the container namespace via nsenter
        run_cmd(&["nsenter", "-t", &pid.to_string(), "--net", "--",
                  "ip", "addr", "add", &gw_cidr, "dev", ceth])?;
        run_cmd(&["nsenter", "-t", &pid.to_string(), "--net", "--",
                  "ip", "link", "set", ceth, "up"])?;
        run_cmd(&["nsenter", "-t", &pid.to_string(), "--net", "--",
                  "ip", "route", "add", "default", "via", gateway])?;

        // Write /etc/resolv.conf inside the container (via bind-mount path in rootfs)
        // This is best-effort — containers that mount their own resolv.conf will override.
        let resolv_content = dns.iter()
            .map(|d| format!("nameserver {d}"))
            .collect::<Vec<_>>()
            .join("\n") + "\n";
        let _ = run_cmd(&["nsenter", "-t", &pid.to_string(), "--mount", "--",
            "sh", "-c", &format!("echo '{resolv_content}' > /etc/resolv.conf")]);

        tracing::info!(
            container_id = %container_id,
            pid = %pid,
            "network: bridge attached"
        );
        Ok(())
    }

    async fn cleanup(&self, container_id: &str) -> Result<()> {
        let prefix = &container_id[..8.min(container_id.len())];
        let host_veth = format!("veth-{prefix}");
        // Delete veth (removing host side also removes peer)
        if let Err(e) = run_cmd(&["ip", "link", "delete", &host_veth]) {
            tracing::warn!(container_id = %container_id, error = %e,
                "network: veth cleanup failed (may already be gone)");
        }
        // Clean up net context file
        let net_file = format!("/run/minibox/net/{container_id}.json");
        let _ = std::fs::remove_file(&net_file);
        Ok(())
    }

    async fn stats(&self, container_id: &str) -> Result<NetworkStats> {
        let prefix = &container_id[..8.min(container_id.len())];
        let veth = format!("veth-{prefix}");
        let read_u64 = |metric: &str| -> u64 {
            std::fs::read_to_string(
                format!("/sys/class/net/{veth}/statistics/{metric}")
            ).ok()
              .and_then(|s| s.trim().parse().ok())
              .unwrap_or(0)
        };
        Ok(NetworkStats {
            rx_bytes:   read_u64("rx_bytes"),
            tx_bytes:   read_u64("tx_bytes"),
            rx_packets: read_u64("rx_packets"),
            tx_packets: read_u64("tx_packets"),
            rx_errors:  read_u64("rx_errors"),
            tx_errors:  read_u64("tx_errors"),
            rx_dropped: read_u64("rx_dropped"),
            tx_dropped: read_u64("tx_dropped"),
        })
    }
}
```

- [ ] **Step 2: Re-export from `network/mod.rs`**

```rust
#[cfg(target_os = "linux")]
pub mod bridge;
#[cfg(target_os = "linux")]
pub use bridge::BridgeNetwork;
```

- [ ] **Step 3: Check**

```bash
cargo check -p minibox
```

Expected: compiles cleanly on Linux. On macOS, `BridgeNetwork` is not compiled.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox/src/adapters/network/bridge.rs crates/minibox/src/adapters/network/mod.rs
git commit -m "feat(network): add BridgeNetwork adapter with veth/NAT setup"
```

---

### Task 15: Write network context file in `setup()`, wire `BridgeNetwork` in `miniboxd`

**Files:**

- Modify: `crates/minibox/src/adapters/network/bridge.rs`
- Modify: `crates/miniboxd/src/main.rs`
- Modify: `crates/daemonbox/src/state.rs`

- [ ] **Step 1: Write net context file in `setup()`**

At the end of `BridgeNetwork::setup()`, before returning the JSON string, add:

```rust
// Write context for attach() to consume
let net_dir = std::path::Path::new("/run/minibox/net");
std::fs::create_dir_all(net_dir)?;
let net_file = net_dir.join(container_id).with_extension("json");
let json = serde_json::to_string(&serde_json::json!({
    "container_ip": container_ip.to_string(),
    "ceth": ceth,
    "gateway": self.ip_alloc.lock().unwrap().gateway().to_string(),
    "dns": self.dns_servers.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
}))?;
std::fs::write(&net_file, &json)?;
Ok(json)
```

- [ ] **Step 2: Add `allocated_ips` to `DaemonState`**

In `crates/daemonbox/src/state.rs`, add to `DaemonState`:

```rust
/// IP addresses currently allocated by bridge network, keyed by container_id.
pub allocated_ips: Arc<RwLock<HashMap<String, std::net::IpAddr>>>,
```

Initialize as `Arc::new(RwLock::new(HashMap::new()))` in `DaemonState::new()`.

- [ ] **Step 3: Wire `BridgeNetwork` in `miniboxd/src/main.rs`**

```rust
#[cfg(target_os = "linux")]
let network_provider: Arc<dyn NetworkProvider> = {
    let mode = std::env::var("MINIBOX_NETWORK_MODE")
        .unwrap_or_else(|_| "none".to_string());
    match mode.as_str() {
        "bridge" => Arc::new(
            minibox::adapters::network::bridge::BridgeNetwork::new()
                .expect("BridgeNetwork init failed")
        ),
        "host" => Arc::new(minibox::adapters::network::host::HostNetwork),
        _ => Arc::new(minibox::adapters::network::none::NoopNetwork),
    }
};
```

Pass `network_provider` through to `HandlerDependencies` (it likely already flows through — verify it reaches `handle_run` → `spawn_process`).

- [ ] **Step 4: Check**

```bash
cargo check -p miniboxd -p minibox -p daemonbox
```

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/src/adapters/network/bridge.rs \
        crates/daemonbox/src/state.rs \
        crates/miniboxd/src/main.rs
git commit -m "feat(network): wire BridgeNetwork in miniboxd via MINIBOX_NETWORK_MODE=bridge"
```

---

### Task 16: Port mapping via iptables DNAT

**Files:**

- Modify: `crates/minibox/src/adapters/network/bridge.rs`

- [ ] **Step 1: Add `apply_port_mappings()` to `BridgeNetwork`**

Add this method after `ensure_nat()`:

```rust
fn apply_port_mappings(
    &self,
    container_ip: &str,
    mappings: &[minibox_core::domain::PortMapping],
) -> Result<()> {
    for pm in mappings {
        let proto = match pm.protocol {
            minibox_core::domain::Protocol::Tcp  => "tcp",
            minibox_core::domain::Protocol::Udp  => "udp",
            minibox_core::domain::Protocol::Sctp => "sctp",
        };
        let dport = pm.host_port.to_string();
        let to_dest = format!("{container_ip}:{}", pm.container_port);

        // Check if rule already exists
        let check = Command::new("iptables")
            .args(["-t", "nat", "-C", "PREROUTING",
                   "-p", proto, "--dport", &dport,
                   "-j", "DNAT", "--to-destination", &to_dest])
            .status()?;
        if !check.success() {
            run_cmd(&["iptables", "-t", "nat", "-A", "PREROUTING",
                      "-p", proto, "--dport", &dport,
                      "-j", "DNAT", "--to-destination", &to_dest])?;
        }
        tracing::info!(
            host_port = pm.host_port,
            container_port = pm.container_port,
            proto = proto,
            "network: port mapping added"
        );
    }
    Ok(())
}
```

Call `self.apply_port_mappings(&container_ip.to_string(), &config.port_mappings)?;` at the end of `setup()` before writing the context file. Also save `port_mappings` to the net context JSON for `cleanup()` to remove them.

- [ ] **Step 2: Remove DNAT rules in `cleanup()`**

In `cleanup()`, after the veth deletion, read the net context file and remove port mappings:

```rust
if let Ok(bytes) = std::fs::read(&net_file) {
    if let Ok(ctx) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if let Some(mappings) = ctx["port_mappings"].as_array() {
            for m in mappings {
                let proto = m["proto"].as_str().unwrap_or("tcp");
                let dport = m["host_port"].to_string();
                let to_dest = format!("{}:{}",
                    m["container_ip"].as_str().unwrap_or(""),
                    m["container_port"]);
                let _ = run_cmd(&["iptables", "-t", "nat", "-D", "PREROUTING",
                                  "-p", proto, "--dport", &dport,
                                  "-j", "DNAT", "--to-destination", &to_dest]);
            }
        }
    }
}
```

- [ ] **Step 3: Check**

```bash
cargo check -p minibox
```

Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox/src/adapters/network/bridge.rs
git commit -m "feat(network): add port mapping via iptables DNAT in BridgeNetwork"
```

---

## Final: Quality Gate and Integration Test

### Task 17: Pre-commit gate and integration test skeleton

**Files:**

- Modify: `crates/minibox/src/adapters/network/bridge.rs` (integration test)

- [ ] **Step 1: Run pre-commit gate**

```bash
cargo xtask pre-commit
```

Expected: fmt check passes, clippy clean, release build succeeds. Fix any warnings before proceeding.

- [ ] **Step 2: Run full unit test suite**

```bash
cargo xtask test-unit
```

Expected: all tests pass (new tests from phases 1-3 included).

- [ ] **Step 3: Add Linux-gated integration test for bridge networking**

In `crates/minibox/src/adapters/network/bridge.rs`, add:

```rust
#[cfg(all(test, target_os = "linux"))]
mod integration_tests {
    use super::*;

    /// Run with: just test-integration (requires root + Linux)
    ///
    /// Verifies BridgeNetwork can create a bridge interface without panicking.
    /// Full attach() test requires a running container — see e2e suite.
    #[tokio::test]
    #[ignore = "requires root and Linux kernel with bridge support"]
    async fn test_bridge_setup_creates_interface() {
        let bridge = BridgeNetwork::new().expect("BridgeNetwork init");
        bridge.ensure_bridge().expect("ensure_bridge");

        // Verify minibox0 exists
        let status = std::process::Command::new("ip")
            .args(["link", "show", "minibox0"])
            .status()
            .unwrap();
        assert!(status.success(), "minibox0 bridge should exist after ensure_bridge()");
    }
}
```

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "test(network): add ignored integration test for BridgeNetwork setup"
```

- [ ] **Step 5: Push**

```bash
git push
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                          | Task    |
| --------------------------------------------------------- | ------- |
| `CgroupManager::pause()`/`resume()`                       | Task 1  |
| `ContainerState::Paused` + transitions                    | Task 2  |
| `PauseContainer`/`ResumeContainer` protocol               | Task 3  |
| `handle_pause`/`handle_resume`                            | Task 4  |
| `minibox pause`/`resume` CLI                              | Task 5  |
| `EventSink`/`EventSource`/`BroadcastEventBroker`          | Task 6  |
| `event_sink` in `HandlerDependencies`, emit at lifecycle  | Task 7  |
| `SubscribeEvents` protocol + `minibox events` CLI         | Task 8  |
| `list_all_images`, `delete_image`, `image_size_bytes`     | Task 9  |
| `DiskLeaseService` + `ImageLeaseService` trait            | Task 10 |
| `ImageGc` + `PruneReport`                                 | Task 11 |
| `Prune`/`RemoveImage` protocol + CLI                      | Task 12 |
| `IpAllocator`                                             | Task 13 |
| `BridgeNetwork::setup()`/`attach()`/`cleanup()`/`stats()` | Task 14 |
| Net context file, `allocated_ips`, miniboxd wiring        | Task 15 |
| Port mapping via iptables                                 | Task 16 |
| Pre-commit gate + integration test                        | Task 17 |

All spec requirements covered. No placeholders. Types are consistent across tasks (`ContainerEvent`, `LeaseRecord`, `PruneReport`, `BridgeNetwork`, `IpAllocator` used consistently throughout).
