# Design: Crux Governed Execution Plugin

## Status

Approved on 2026-09-22. Implementation still requires a separate plan and depends on the approved
Crux envelope and Minibox governed-daemon designs.

## Goal

Expose Minibox governed execution as one typed Crux plugin handler without signing, weakening, or
reconstructing the caller's authorization.

## Approved Approach

Add `minibox::execution::run`; it accepts an externally signed `ExecutionEnvelopeV1`, forwards it
without semantic mutation, consumes a bounded daemon response sequence, and returns the completed
envelope. V1 intentionally does not stream progress through the current single-response plugin
protocol.

## Context Map

### Files To Modify

| File                                              | Purpose                | Change                                                                     |
| ------------------------------------------------- | ---------------------- | -------------------------------------------------------------------------- |
| `Cargo.toml`                                      | Workspace dependencies | Add the pinned `crux-types` source used by `minibox-core`.                 |
| `crates/minibox-crux-plugin/Cargo.toml`           | Plugin dependencies    | Depend directly on `crux-types`.                                           |
| `crates/minibox-crux-plugin/src/lib.rs`           | Handler and dispatch   | Register and dispatch governed execution.                                  |
| `crates/minibox-crux-plugin/src/protocol.rs`      | Raw request parsing    | Reject duplicate JSON object keys before constructing `serde_json::Value`. |
| `crates/minibox-crux-plugin/src/main.rs`          | Stdio boundary         | Run duplicate-key and frame-size validation on each raw line.              |
| `crates/minibox-crux-plugin/tests/integration.rs` | Integration coverage   | Cover forwarding, bounded progress consumption, rejection, and completion. |

### Dependencies

| File                                         | Relationship                               |
| -------------------------------------------- | ------------------------------------------ |
| `crates/minibox-crux-plugin/src/protocol.rs` | Existing plugin framing remains unchanged. |
| `crates/minibox-core/src/protocol.rs`        | Supplies governed daemon variants.         |
| `crates/crux-types/src/execution/mod.rs`     | Supplies canonical envelope and records.   |

### Risk

- Existing plugin input is untyped `serde_json::Value`; governed input must deserialize once and
  reject all invalid data.
- Governed execution must consume bounded frames without collecting progress payloads in memory.
- The plugin must never add, remove, reorder, or re-sign records.

## Crate Ownership

- **Owner**: `minibox-crux-plugin` owns discovery, decoding, daemon transport, and encoding.
- **Not owner**: policy, signing, verification, nonce protection, execution, persistence, and
  recovery remain outside the plugin.

## Public API

```rust
pub const GOVERNED_EXECUTION_HANDLER: &str = "minibox::execution::run";

pub fn build_governed_request(
    input: &serde_json::Value,
) -> Result<DaemonRequest>;

pub async fn dispatch_governed(
    envelope: ExecutionEnvelopeV1,
) -> Result<ExecutionEnvelopeV1, GovernedDispatchError>;

pub enum GovernedDispatchError {
    InvalidEnvelope { message: String },
    DaemonRejected { code: String, message: String },
    ResponseLimitExceeded,
    PrematureClose,
    IdentityMismatch,
    InvalidCompletion { message: String },
    Transport { message: String },
}
```

The declaration advertises `ExecutionEnvelopeV1` input and output and side effects constrained by
the signed grant. Machine-readable schema digests remain unavailable until the general Crux plugin
protocol is versioned.

## Data Flow

1. At the raw stdio boundary, reject frames over 1 MiB and any duplicate JSON object key before
   constructing the plugin request or `serde_json::Value`.
2. Deserialize into `ExecutionEnvelopeV1` and validate the neutral lifecycle prefix.
3. Require an authorization record but leave signature verification to Minibox.
4. Send `DaemonRequest::ExecuteGoverned` unchanged.
5. Validate and discard accepted/record progress frames after checking execution identity, sequence,
   per-frame size, total frame count, and total response bytes.
6. Stop at the first terminal response, matching the existing daemon stream contract, and retain
   only the terminal envelope or rejection.
7. Return the completed envelope or typed daemon rejection.

## Failure Rules

- Invalid JSON, lifecycle, or schema fails before transport.
- Daemon rejection retains stable code and message fields internally. The unchanged plugin wire
  emits `InvokeErr.error` as compact JSON text `{"code":"...","message":"..."}`.
- Premature stream close is an error; the plugin never synthesizes a result.
- A terminal envelope retains the submitted execution ID and intent digest.
- Requests and individual frames are limited to 1 MiB; an exchange permits at most 256 record
  frames and 2 MiB total response bytes.

## Out Of Scope

- Generating envelopes from legacy run input.
- Signing or refreshing authorization.
- Falling back to ordinary `DaemonRequest::Run`.
- Changing existing container and image handlers.
- Redesigning the general Crux subprocess plugin protocol.

## Acceptance Criteria

- The daemon receives a structurally equivalent envelope with identical execution ID, intent digest,
  authorization proof, and existing record sequence.
- Invalid or unsigned envelopes never become ordinary runs.
- Bounded record responses are consumed in order and completion returns a valid envelope.
- Premature close, changed execution ID, or changed intent fails.
- Existing plugin handlers retain current behavior.
