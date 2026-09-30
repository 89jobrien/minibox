---
source_sha: f9aa9a79348227ca3c517fe7ea86b3bc6eb16829
status: proposed
sources:
  - crates/minibox-core/src/protocol.rs
  - crates/minibox-core/tests/protocol_evolution.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox/src/daemon/server.rs
  - crates/minibox/src/daemon/state.rs
  - crates/mbx/src/main.rs
  - crates/mbx/src/commands/run.rs
  - crates/mcp/src/client.rs
  - crates/mcp/src/tools/containers.rs
  - crates/minibox-crux-plugin/src/lib.rs
  - crates/macbox/src/vz/proxy.rs
  - docs/designs/2026-09-21-isolation-profile-contract-design.md
  - docs/designs/2026-09-21-isolation-admission-lifecycle-design.md
generated: 2026-09-21
---

# Design: Isolation Protocol And Migration

## Goal

Add profile negotiation, structured denial, accepted evidence summaries, and manifest v2 without
allowing new clients to downgrade silently against old daemons.

## Handshake

Every new client performs `GetIsolationCapabilities` before a run request.

```rust
pub struct IsolationServiceCapabilities {
    protocol_version: u32,
    default_profile: IsolationProfileId,
    compatibility_authorized: bool,
    runtime: IsolationRuntimeCapabilities,
}

pub enum DaemonRequest {
    GetIsolationCapabilities,
    // Existing variants.
}

pub enum DaemonResponse {
    IsolationCapabilities {
        service: IsolationServiceCapabilities,
    },
    IsolationDenied {
        denial: IsolationDenial,
    },
    // Existing variants.
}
```

`IsolationCapabilities` and `IsolationDenied` are terminal. If an old daemon rejects or cannot
answer the handshake, the client does not send a workload.

## Run Request

`DaemonRequest::Run` retains legacy `privileged: bool` and adds:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
isolation: Option<IsolationProfileRequest>
```

Omitted isolation normalizes to `standard-v1`. `privileged=true` conflicts with profile selection,
`cap-add`, and `cap-drop` in both clients and daemon admission.

## Accepted Response

`ContainerCreated` remains non-terminal and gains:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
isolation: Option<ExecutionIsolationSummary>
```

The summary contains status, profile/mode, policy digest, plan digest, evidence digest, and runtime
identity. New clients reject a missing summary after a successful isolation handshake.

Cleanup failure after creation is reported by terminal `ContainerStopped` with a structured cleanup
status and optional cleanup denial. It does not introduce a competing terminal response. For
non-ephemeral runs, cleanup state remains queryable from persisted container state.

## Structured Denial

```rust
pub enum IsolationDenialCode {
    InvalidRequest,
    ProfileNotAuthorized,
    UnsupportedProfile,
    CapabilityNotAuthorized,
    UnsupportedCapability,
    UnsupportedControl,
    LegacyPrivilegedConflict,
    LegacyPrivilegedNotAuthorized,
    InvalidPlan,
    StalePlan,
    IncompleteEvidence,
    EnforcementFailed,
    CleanupFailed,
}
```

Denials include profile, control, adapter, and a non-secret human message where applicable.

## CLI

`mbx run` adds:

- Repeatable `--cap-add <CAPABILITY>`.
- Repeatable `--cap-drop <CAPABILITY>`.
- Single `--isolation-profile <standard-v1|compatibility-v1>`.

Clap rejects incompatible privileged/profile combinations before transport. CLI output prints the
accepted profile and abbreviated evidence digest.

## Client Parity

The following clients must handshake and preserve the same request semantics:

- `mbx`
- MCP container tools
- `minibox-crux-plugin`
- VZ host/proxy path
- Pipeline/sandbox clients

MCP, Crux, and pipeline policy may narrow requests but cannot authorize compatibility, privileged
mode, host mounts, host networking, or capability additions beyond daemon policy.

## Manifest V2

Manifest v1 digest semantics remain byte-for-byte frozen and gain a golden fixture before changes.
V2 adds `ExecutionIsolationRecord` with:

- Mode and status.
- Policy, plan, binding, evidence, and cleanup digests.
- Runtime identity/environment generation.
- Operator authorization where required.
- Degraded controls for compatibility.

Historical v1 manifests remain readable but unqualified. They are never upgraded into evidence
claims.

## Status Model

Profile execution:

```text
planned -> staged -> enforced -> exited -> cleaned
```

Legacy execution:

```text
legacy-authorized -> legacy-running -> exited -> cleaned
```

Any state may transition to `failed-cleanup-pending`, which persists a cleanup handle and retries
until `cleaned`. Unknown and illegal transitions fail closed.

## Legacy Privileged Migration

Existing `allow_privileged=true` and `MINIBOX_ALLOW_PRIVILEGED=true` inputs remain accepted during a
deprecation window. Daemon config normalization converts the source and config digest into an
`OperatorAuthorizationId`. New protocol/state code uses that ID; bare booleans cannot reach runtime
adapters.

Legacy privileged remains separate from `compatibility-v1`. Neither authorizes the other.

## Compatibility Migration

Compatibility requires:

1. Explicit caller profile selection.
2. Operator authorization.
3. Complete enforced/degraded accounting for all controls.
4. Authorization ID in plan, evidence, manifest, and response summary.

There is no automatic fallback from standard to compatibility.

## Backward Compatibility Matrix

| Client | Daemon | Result                                                                                          |
| ------ | ------ | ----------------------------------------------------------------------------------------------- |
| New    | New    | Handshake, then profile-aware run                                                               |
| New    | Old    | Handshake fails; no workload request                                                            |
| Old    | New    | Omitted fields normalize to standard; old client may not understand summary but daemon enforces |
| Old    | Old    | Existing behavior; outside new guarantee                                                        |

## Drift And Fixtures

- Update `taskit protocol drift` tracked surfaces.
- Add old/new request and response fixtures.
- Add terminal-response canonical-table tests.
- Add serde defaults for additive fields.
- Add exhaustive client match tests.
- Pin manifest v1/v2 golden vectors.

## Out Of Scope

- Kernel enforcement details.
- Guest qualification.
- Signed policies or remote attestation.
