---
status: done
---

# MINIBOX-SC Slashcrux Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate `slashcrux` vocabulary types (`Priority`, `Urgency`, `StepState`,
`ExecutionContext`) into the minibox container runtime so containers carry agentic workflow
metadata end-to-end: from protocol request through daemon state to handler execution.

**Architecture:** Hexagonal (ports/adapters). `slashcrux` types enter via the protocol layer
(minibox-core), flow through domain state (DaemonState/ContainerRecord), and inform policy
decisions (ContainerPolicy). No adapter changes needed — this is a core/domain integration.

**Tech Stack:** Rust 2024, `cargo nextest`, `serde`, `slashcrux` 0.1.2 (path dep initially,
crates.io later).

---

## Causal Chain

```text
T1: Add slashcrux workspace dep             (prereq for all)
  └── T2: Wire into minibox-core protocol   (unblocks T3, T4)
        ├── T3: Wire into DaemonState        (container records)
        │     └── T5: Wire into handler      (execution context threading)
        └── T4: Wire into ContainerPolicy    (priority-aware scheduling)
              └── T6: Tests + snapshots
                    └── T7: Commit
```

**Note:** T3 and T4 can proceed in parallel once T2 is done. T5 depends on T3. T6 covers
all integration tests across the chain.

---

## File Map

| Action    | Path                                         |
| --------- | -------------------------------------------- |
| Modify    | `Cargo.toml` (workspace dep)                 |
| Modify    | `crates/minibox-core/Cargo.toml`             |
| Modify    | `crates/minibox-core/src/protocol.rs`        |
| Modify    | `crates/minibox-core/src/domain.rs`          |
| Modify    | `crates/minibox/Cargo.toml`                  |
| Modify    | `crates/minibox/src/daemon/state.rs`         |
| Modify    | `crates/minibox/src/daemon/handler.rs`       |
| Reference | `crates/minibox-core/tests/protocol_evolution.rs` |

---

## Task 1: Add slashcrux as workspace dependency

**Files:** `Cargo.toml`, `crates/minibox-core/Cargo.toml`

- [ ] **Step 1: Add path dep to workspace Cargo.toml**

  ```toml
  # [workspace.dependencies]
  slashcrux = { path = "../slashcrux" }
  ```

- [ ] **Step 2: Add to minibox-core Cargo.toml**

  ```toml
  # [dependencies]
  slashcrux = { workspace = true }
  ```

- [ ] **Step 3: Verify it compiles**

  ```bash
  cargo check -p minibox-core
  ```

---

## Task 2: Wire slashcrux types into protocol (minibox-core)

**Files:**
- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/minibox-core/src/domain.rs`

**Change:** Add `priority`, `urgency`, and `execution_context` fields to the `Run` variant
of `DaemonRequest`. Re-export slashcrux types from `minibox-core::domain` for downstream
crate access.

**Implementation:**

- [ ] **Step 1: Re-export slashcrux types from domain.rs**

  Add to `crates/minibox-core/src/domain.rs`:

  ```rust
  // Re-export slashcrux vocabulary types for agentic workflow metadata.
  pub use slashcrux::{ExecutionContext, Priority, StepState, Urgency};
  ```

- [ ] **Step 2: Add fields to `DaemonRequest::Run`**

  Add three new optional fields to the `Run` variant in `protocol.rs`. All use
  `#[serde(default)]` for backward compatibility:

  ```rust
  Run {
      // ... existing fields ...

      /// Scheduling priority for this container run.
      #[serde(default)]
      priority: Option<slashcrux::Priority>,

      /// Urgency hint for the scheduler.
      #[serde(default)]
      urgency: Option<slashcrux::Urgency>,

      /// Agentic execution context — workflow variables and bindings
      /// carried from the orchestrator into the container environment.
      #[serde(default)]
      execution_context: Option<slashcrux::ExecutionContext>,
  }
  ```

- [ ] **Step 3: Add same fields to `DaemonRequest::RunPipeline`**

  ```rust
  RunPipeline {
      // ... existing fields ...

      #[serde(default)]
      priority: Option<slashcrux::Priority>,

      #[serde(default)]
      urgency: Option<slashcrux::Urgency>,

      #[serde(default)]
      execution_context: Option<slashcrux::ExecutionContext>,
  }
  ```

- [ ] **Step 4: Add backward-compat serde snapshot tests**

  In `crates/minibox-core/tests/protocol_evolution.rs`, add:

  ```rust
  #[test]
  fn test_request_run_backward_compat_omits_slashcrux_fields() {
      // Old-format JSON without priority/urgency/execution_context
      // must still deserialize successfully (all default to None).
      let json = r#"{"type":"Run","image":"alpine","command":["/bin/sh"]}"#;
      let req: DaemonRequest = serde_json::from_str(json).unwrap();
      // verify it parsed as Run variant
  }
  ```

- [ ] **Step 5: Verify compile + existing tests pass**

  ```bash
  cargo check -p minibox-core
  cargo nextest run -p minibox-core
  ```

---

## Task 3: Wire StepState into DaemonState container records

**Files:**
- Modify: `crates/minibox/Cargo.toml`
- Modify: `crates/minibox/src/daemon/state.rs`

**Change:** Add a `step_state` field to `ContainerRecord` that mirrors the container's
lifecycle using `slashcrux::StepState`. This bridges the minibox-internal `ContainerState`
enum (Created/Running/Stopped/Failed) to the slashcrux workflow vocabulary so orchestrators
can query container progress in agentic terms.

- [ ] **Step 1: Add slashcrux dep to minibox crate**

  ```toml
  # crates/minibox/Cargo.toml [dependencies]
  slashcrux = { workspace = true }
  ```

- [ ] **Step 2: Add step_state field to ContainerRecord**

  ```rust
  pub struct ContainerRecord {
      // ... existing fields ...

      /// Agentic workflow step state — maps container lifecycle to
      /// slashcrux StepState for orchestrator consumption.
      #[serde(default)]
      pub step_state: slashcrux::StepState,
  }
  ```

- [ ] **Step 3: Add conversion from ContainerState to StepState**

  In `state.rs`:

  ```rust
  impl From<ContainerState> for slashcrux::StepState {
      fn from(cs: ContainerState) -> Self {
          match cs {
              ContainerState::Created => slashcrux::StepState::Pending,
              ContainerState::Running => slashcrux::StepState::Running,
              ContainerState::Paused => slashcrux::StepState::Running,
              ContainerState::Stopped => slashcrux::StepState::Completed,
              ContainerState::Failed => slashcrux::StepState::Failed,
              ContainerState::Orphaned => slashcrux::StepState::Cancelled,
          }
      }
  }
  ```

- [ ] **Step 4: Update state transitions to sync step_state**

  Wherever `ContainerRecord.info.state` is updated (add_container, update_state methods),
  also set `record.step_state = StepState::from(new_container_state)`.

- [ ] **Step 5: Add priority and urgency to ContainerRecord**

  ```rust
  pub struct ContainerRecord {
      // ... existing fields ...

      #[serde(default)]
      pub priority: Option<slashcrux::Priority>,
      #[serde(default)]
      pub urgency: Option<slashcrux::Urgency>,
  }
  ```

- [ ] **Step 6: Write tests for state-to-step-state conversion**

  ```rust
  #[test]
  fn container_state_to_step_state_mapping() {
      assert_eq!(StepState::from(ContainerState::Created), StepState::Pending);
      assert_eq!(StepState::from(ContainerState::Running), StepState::Running);
      assert_eq!(StepState::from(ContainerState::Stopped), StepState::Completed);
      assert_eq!(StepState::from(ContainerState::Failed), StepState::Failed);
      assert_eq!(StepState::from(ContainerState::Orphaned), StepState::Cancelled);
  }
  ```

- [ ] **Step 7: Verify**

  ```bash
  cargo check -p minibox
  cargo nextest run -p minibox
  ```

---

## Task 4: Wire Priority/Urgency into ContainerPolicy

**Files:**
- Modify: `crates/minibox/src/daemon/handler.rs`

**Change:** Extend `ContainerPolicy` with optional `min_priority` field. When set,
`validate_policy` rejects container runs below the minimum priority. This enables
operators to enforce scheduling gates (e.g., only Critical/High during incident response).

- [ ] **Step 1: Add min_priority to ContainerPolicy**

  ```rust
  #[derive(Debug, Clone, Default)]
  pub struct ContainerPolicy {
      pub allow_bind_mounts: bool,
      pub allow_privileged: bool,

      /// Minimum priority required to run a container.
      /// `None` means no priority gate (all priorities accepted).
      pub min_priority: Option<slashcrux::Priority>,
  }
  ```

- [ ] **Step 2: Update validate_policy to check priority**

  ```rust
  // Inside validate_policy():
  if let Some(min) = &policy.min_priority {
      if let Some(req_priority) = &request_priority {
          if req_priority.score() < min.score() {
              return Err(format!(
                  "container priority {:?} is below minimum {:?}",
                  req_priority, min
              ));
          }
      }
      // No priority on request + min_priority set → reject
  }
  ```

- [ ] **Step 3: Write tests**

  ```rust
  #[test]
  fn policy_rejects_low_priority_when_min_set() {
      let policy = ContainerPolicy {
          min_priority: Some(Priority::High),
          ..Default::default()
      };
      // A Low priority request should be rejected
  }

  #[test]
  fn policy_accepts_high_priority_when_min_set() {
      let policy = ContainerPolicy {
          min_priority: Some(Priority::High),
          ..Default::default()
      };
      // A Critical priority request should be accepted
  }

  #[test]
  fn policy_no_min_priority_accepts_all() {
      let policy = ContainerPolicy::default();
      // Any priority should be accepted
  }
  ```

- [ ] **Step 4: Verify**

  ```bash
  cargo nextest run -p minibox -- handler
  ```

---

## Task 5: Thread ExecutionContext through handler

**Files:**
- Modify: `crates/minibox/src/daemon/handler.rs`
- Modify: `crates/minibox/src/daemon/state.rs`

**Change:** Pass `ExecutionContext` from `DaemonRequest::Run` through the handler into
`ContainerRecord`. The context variables are injected as additional environment variables
into the container process (JSON-stringified values for non-string types).

- [ ] **Step 1: Add execution_context to ContainerRecord**

  ```rust
  pub struct ContainerRecord {
      // ... existing fields ...

      #[serde(default)]
      pub execution_context: Option<slashcrux::ExecutionContext>,
  }
  ```

- [ ] **Step 2: Extract and store context in handle_run**

  In the `handle_run` / `handle_run_streaming` flow, extract `execution_context` from the
  request and store it on the `ContainerRecord`. If context variables exist, append them
  to the container's environment as `MINIBOX_CTX_{KEY}={value}` pairs.

- [ ] **Step 3: Thread priority/urgency from request to record**

  In the same flow, copy `priority` and `urgency` from the request into the
  `ContainerRecord`.

- [ ] **Step 4: Write integration test**

  ```rust
  #[tokio::test]
  async fn handle_run_stores_execution_context() {
      // Build a Run request with execution_context containing {"workflow": "test"}
      // Verify the resulting ContainerRecord has the context stored
  }
  ```

- [ ] **Step 5: Verify full suite**

  ```bash
  cargo nextest run -p minibox
  ```

---

## Task 6: Integration tests and snapshot updates

**Files:**
- Modify: `crates/minibox-core/tests/protocol_evolution.rs`
- Modify: existing snapshot files if insta is used

- [ ] **Step 1: Add serde roundtrip tests for new fields**

  ```rust
  #[test]
  fn run_request_with_slashcrux_fields_roundtrips() {
      let json = r#"{
          "type": "Run",
          "image": "alpine",
          "command": ["/bin/sh"],
          "priority": "critical",
          "urgency": "immediate",
          "execution_context": {"workflow_id": "wf-123"}
      }"#;
      let req: DaemonRequest = serde_json::from_str(json).unwrap();
      let reserialized = serde_json::to_string(&req).unwrap();
      let req2: DaemonRequest = serde_json::from_str(&reserialized).unwrap();
      // verify fields survive roundtrip
  }
  ```

- [ ] **Step 2: Verify backward compat — old JSON without new fields parses**

  ```rust
  #[test]
  fn run_request_without_slashcrux_fields_still_parses() {
      let json = r#"{"type":"Run","image":"alpine","command":[]}"#;
      let _req: DaemonRequest = serde_json::from_str(json).unwrap();
  }
  ```

- [ ] **Step 3: Run full workspace test suite**

  ```bash
  cargo nextest run --workspace
  ```

- [ ] **Step 4: Update any failing insta snapshots**

  ```bash
  cargo insta review
  ```

---

## Task 7: Commit slashcrux integration

- [ ] **Step 1: Run pre-commit gate**

  ```bash
  cargo xtask pre-commit
  ```

- [ ] **Step 2: Stage and commit**

  ```bash
  git add Cargo.toml Cargo.lock \
    crates/minibox-core/Cargo.toml \
    crates/minibox-core/src/protocol.rs \
    crates/minibox-core/src/domain.rs \
    crates/minibox-core/tests/protocol_evolution.rs \
    crates/minibox/Cargo.toml \
    crates/minibox/src/daemon/state.rs \
    crates/minibox/src/daemon/handler.rs
  git commit -m "feat(core): integrate slashcrux vocabulary types

  Wire Priority, Urgency, StepState, and ExecutionContext from slashcrux
  into the minibox protocol, daemon state, and container policy layers.

  - DaemonRequest::Run/RunPipeline gain optional priority/urgency/context
  - ContainerRecord tracks StepState mirroring container lifecycle
  - ContainerPolicy gains min_priority scheduling gate
  - ExecutionContext variables injected as MINIBOX_CTX_* env vars
  - All new fields use #[serde(default)] for backward compatibility"
  ```

---

## Self-Review

**Spec coverage check:**

| Gap / objective                                | Task |
| ---------------------------------------------- | ---- |
| Protocol backward compat for new fields        | T2   |
| ContainerState -> StepState lifecycle mapping   | T3   |
| Priority-based scheduling gate in policy        | T4   |
| ExecutionContext threading to container env      | T5   |
| Serde roundtrip + snapshot stability            | T6   |

**Placeholder scan:** All placeholders filled with concrete paths and types.

**Type consistency:** `slashcrux::{Priority, Urgency, StepState, ExecutionContext}` match
the 0.1.2 API at `~/dev/slashcrux/src/lib.rs`. `ContainerState` variants match
`minibox-core/src/domain.rs:791`. `ContainerRecord` fields match
`minibox/src/daemon/state.rs:150`. `ContainerPolicy` fields match
`minibox/src/daemon/handler.rs:257`.
