# Design: Signed Governed Execution Daemon

## Status

Approved on 2026-09-22. Implementation still requires a separate plan and depends on the approved
Crux `ExecutionEnvelopeV1` contract.

## Goal

Add one Minibox daemon operation that verifies an externally signed Crux execution envelope,
prevents authorization replay, enforces granted and local constraints, executes the workload, and
returns a durable completed envelope.

## Approved Approach

Use Ed25519 authorization proofs and a local allowlisted trust store. External authorization is
necessary but not sufficient: existing Minibox policies remain non-bypassable controls.

## Relationship To Existing Designs

This design supersedes the execution-record shape in
`2026-08-26-daemon-native-verified-execution-design.md` for Crux-governed calls. It does not claim
the hostile isolation proposed by `2026-09-18-environment-attested-isolation-design.md`. Future
environment-attested evidence can be referenced by the same envelope.

## Context Map

### Files To Modify

| File                                                   | Purpose                  | Change                                                                                |
| ------------------------------------------------------ | ------------------------ | ------------------------------------------------------------------------------------- |
| `Cargo.toml`                                           | Workspace dependencies   | Add pinned `crux-types`, Ed25519, and zeroization dependencies.                       |
| `crates/minibox-core/Cargo.toml`                       | Protocol dependency      | Depend on `crux-types`.                                                               |
| `crates/minibox-core/src/protocol.rs`                  | Daemon wire protocol     | Add governed request and response variants.                                           |
| `crates/minibox-core/src/client/socket.rs`             | Framed daemon client     | Enforce governed request and response frame ceilings.                                 |
| `crates/minibox-core/tests/protocol_evolution.rs`      | Wire snapshots           | Cover governed variants and terminal classification.                                  |
| `crates/minibox-core/tests/property_roundtrip.rs`      | Protocol properties      | Round-trip governed values.                                                           |
| `crates/minibox-domain/src/runtime.rs`                 | Runtime port             | Add optional staged spawn/release operations; non-native adapters report unsupported. |
| `crates/minibox-testsuite/src/adapters/runtime.rs`     | Runtime conformance      | Prove native staged execution and non-native fail-closed defaults.                    |
| `crates/minibox/src/daemon/governed/mod.rs`            | Application service      | Verify, reserve, execute, record, finalize, and recover runs.                         |
| `crates/minibox/src/daemon/governed/authorization.rs`  | Verification adapter     | Verify Ed25519 proofs against the trust store.                                        |
| `crates/minibox/src/daemon/governed/store.rs`          | Durable stores           | Persist envelopes and claim nonces atomically.                                        |
| `crates/minibox/Cargo.toml`                            | Application dependencies | Add direct Crux wire and Ed25519 verification dependencies.                           |
| `crates/minibox/src/daemon/mod.rs`                     | Daemon modules           | Declare the governed execution module.                                                |
| `crates/minibox/src/daemon/handler/mod.rs`             | Handler dependencies     | Expose focused cloneable run dependencies to the governed factory.                    |
| `crates/minibox/src/daemon/handler/run/model.rs`       | Reusable run state       | Extract prepared and running governed state.                                          |
| `crates/minibox/src/daemon/handler/run/request.rs`     | Request normalization    | Share strict normalization with governed execution.                                   |
| `crates/minibox/src/daemon/handler/run/preparation.rs` | Resource preparation     | Accept a durable ownership selector and journal resource locators.                    |
| `crates/minibox/src/daemon/handler/run.rs`             | Shared run engine        | Delegate ordinary and governed execution to extracted run phases.                     |
| `crates/minibox/src/daemon/handler/lifecycle.rs`       | Shared cleanup           | Expose crate-private cleanup by governed ownership selector.                          |
| `crates/minibox/src/container/process.rs`              | Process spawn            | Add a closed execution latch and explicit release acknowledgement.                    |
| `crates/minibox/src/adapters/runtime.rs`               | Native runtime adapter   | Implement staged spawn and release for native Linux.                                  |
| `crates/minibox/src/daemon/state.rs`                   | Recovery linkage         | Persist execution-to-container identity for cleanup-only recovery.                    |
| `crates/minibox/src/daemon/server.rs`                  | Request dispatch         | Reject duplicate raw JSON keys, route governed requests, and stream responses.        |
| `crates/miniboxd/src/config.rs`                        | Daemon configuration     | Add trust-store, audience, evidence-root, and retention settings.                     |
| `crates/miniboxd/src/main.rs`                          | Composition root         | Build adapters and reconcile interrupted executions.                                  |

### Dependencies

| File                                                   | Relationship                                             |
| ------------------------------------------------------ | -------------------------------------------------------- |
| `crates/minibox-domain/src/execution_manifest.rs`      | Supplies workload digest and native manifest evidence.   |
| `crates/minibox-domain/src/execution_policy.rs`        | Supplies local manifest admission checks.                |
| `crates/minibox/src/daemon/handler/run/preparation.rs` | Existing preparation reused after durable authorization. |
| `crates/minibox/src/daemon/handler/run.rs`             | Existing spawn and output streaming reused.              |
| `crates/minibox/src/daemon/handler/lifecycle.rs`       | Existing cleanup path reused before finalization.        |

### Risk

- Additive protocol variants break external exhaustive Rust matches.
- Verification and ledger failures must fail closed.
- A claimed nonce is single-use even when execution fails before spawn.
- Ordinary `Run` remains backward compatible and is never a fallback.
- This proves authorization and evidence continuity, not hostile isolation.

## Crate Ownership

- **Domain owner**: `minibox-domain` owns the staged-runtime port extension with an unsupported
  default for non-native adapters.
- **Protocol owner**: `minibox-core` owns governed request and response DTOs.
- **Application owner**: `minibox` owns verification, stores, orchestration, and mapping.
- **Composition owner**: `miniboxd` owns trust roots, audience, paths, and recovery wiring.
- **Conformance owner**: `minibox-testsuite` owns reusable staged-runtime adapter contracts.

No Minibox domain type depends on Crux. The protocol carries `ExecutionEnvelopeV1`; the application
service maps its action input to existing Minibox domain values. The daemon server owns an optional
type-erased governed executor, configured through a builder; `HandlerDependencies` does not contain
itself or become generic, and existing adapter suites remain source compatible.

Governed V1 is native-Linux-only. `miniboxd` constructs the governed handle only for the native
adapter suite and rejects or disables governed configuration for Colima, GKE, SmolVM, Krun, VZ,
Windows, and other suites until the `ContainerRuntime` port supports staged spawn and release.

## Supported V1 Action

The first executor accepts exactly `minibox.container/run.v1`. Its input deserializes strictly:

```rust
pub struct GovernedRunInputV1 {
    pub image: String,
    pub image_manifest_digest: ContentDigestV1,
    pub command: Vec<String>,
    pub mounts: Vec<BindMount>,
    pub name: Option<String>,
    pub platform: Option<String>,
}
```

The DTO uses `#[serde(deny_unknown_fields)]`. Unknown fields and wrong types are rejected before
reservation. Mount source paths and access must be covered by filesystem grants. Memory, CPU,
process, wall-time, output, network, privilege, environment, and secret authority come exclusively
from the signed grant, not duplicated input fields. Minibox V1 rejects network allowlists,
`Unrestricted` network, privileged execution, non-empty custom environment names, and non-empty
secret grants because it cannot yet enforce those neutral capabilities without ambiguity.

### Normative Mapping

| Minibox value                                                   | Signed source                            | V1 rule                                                                                              |
| --------------------------------------------------------------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Image reference, image manifest digest, command, name, platform | `GovernedRunInputV1`                     | Parsed strictly and bound by the intent digest; resolved content must match exactly.                 |
| Bind mounts                                                     | Input plus filesystem grant              | Every source is within a granted root; requested access matches the grant; target is absolute.       |
| Network                                                         | `CapabilitySetV1.network`                | Only `Denied` is accepted and maps to `NetworkMode::None`.                                           |
| Privilege                                                       | `CapabilitySetV1.process`                | `privileged` must be false.                                                                          |
| Memory, CPU, processes                                          | `CapabilitySetV1.resources`              | Memory and process limits must be finite; CPU weight is copied exactly when present.                 |
| Timeout                                                         | `CapabilitySetV1.resources.wall_time_ms` | Must be finite, nonzero, and within the daemon ceiling.                                              |
| Output                                                          | `CapabilitySetV1.resources.output_bytes` | Must be finite and within the daemon ceiling; `max_inline_bytes` may only narrow inline observation. |
| Environment and secrets                                         | Capability set                           | Both must be empty in Minibox V1.                                                                    |

## Public API

### Protocol

```rust
pub enum DaemonRequest {
    ExecuteGoverned { envelope: ExecutionEnvelopeV1 },
    // existing variants unchanged
}

pub enum DaemonResponse {
    GovernedExecutionAccepted { execution_id: ExecutionId },
    GovernedExecutionRecord {
        execution_id: ExecutionId,
        record: ExecutionRecordV1,
    },
    GovernedExecutionComplete { envelope: ExecutionEnvelopeV1 },
    GovernedExecutionRejected {
        execution_id: Option<ExecutionId>,
        code: String,
        message: String,
    },
    // existing variants unchanged
}
```

Complete and rejected responses are terminal. Accepted and record responses are nonterminal.

### Authorization

```rust
pub(crate) struct VerifiedAuthorization {
    execution_id: ExecutionId,
    key_id: String,
    nonce: AuthorizationNonce,
    expires_at: DateTime<Utc>,
    grant: CapabilitySetV1,
}

pub(crate) trait AuthorizationVerifier: Send + Sync {
    fn verify(
        &self,
        envelope: &ExecutionEnvelopeV1,
        expected_audience: &str,
        now: DateTime<Utc>,
    ) -> Result<VerifiedAuthorization, AuthorizationVerificationError>;
}

pub(crate) struct AuthorizationTrustStore {
    schema_version: u32,
    keys: Vec<TrustedAuthorizationKey>,
}

pub(crate) struct TrustedAuthorizationKey {
    key_id: String,
    public_key: String,
    authority_ids: Vec<String>,
    policy_digests: Vec<ContentDigestV1>,
    valid_from: DateTime<Utc>,
    valid_until: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

pub(crate) struct Ed25519AuthorizationVerifier {
    // private trust-store state
}

pub(crate) enum AuthorizationVerificationError {
    InvalidEnvelope { reason: String },
    MissingAuthorization,
    NonExecutableDecision,
    UnknownKey { key_id: String },
    AuthorityNotTrusted { key_id: String, authority: String },
    PolicyNotTrusted { key_id: String, policy_digest: String },
    AudienceMismatch,
    NotYetValid,
    Expired,
    DigestMismatch,
    InvalidSignature,
    CapabilityEscalation,
}
```

Trust stores contain public keys only, reject duplicate IDs and aliased key material, and load at
daemon startup. Loading uses descriptor-relative opens, rejects symlinks and non-regular files,
requires a trusted owner, owner-only write permissions, and non-writable ancestor directories.
Recovery uses the authorization and key-validity snapshot recorded at reservation time; revocation
blocks new reservations but does not make cleanup impossible.

### Transactional Ledger

```rust
pub(crate) struct AuthorizationClaim {
    execution_id: ExecutionId,
    key_id: String,
    nonce: AuthorizationNonce,
    expires_at: DateTime<Utc>,
}

pub(crate) struct GovernedReservation {
    reservation_id: String,
    execution_id: ExecutionId,
    ownership_selector: String,
    head_digest: ContentDigestV1,
}

pub(crate) struct GovernedResourceLocator {
    kind: String,
    value: String,
    generation: Option<String>,
}

pub(crate) struct GovernedResourcePlan {
    kind: String,
    deterministic_id: String,
    ownership_selector: String,
}

#[async_trait]
pub(crate) trait GovernedExecutionLedger: Send + Sync {
    async fn reserve(
        &self,
        claim: &AuthorizationClaim,
        envelope: &ExecutionEnvelopeV1,
        resources: &[GovernedResourcePlan],
    ) -> Result<GovernedReservation, GovernedLedgerError>;
    async fn compare_and_append(
        &self,
        reservation: &GovernedReservation,
        expected_head: &ContentDigestV1,
        envelope: &ExecutionEnvelopeV1,
    ) -> Result<GovernedReservation, GovernedLedgerError>;
    async fn record_locator(
        &self,
        reservation: &GovernedReservation,
        locator: &GovernedResourceLocator,
    ) -> Result<(), GovernedLedgerError>;
    async fn interrupted(&self) -> Result<Vec<GovernedReservation>, GovernedLedgerError>;
    async fn prune_tombstones(&self, now: DateTime<Utc>) -> Result<u64, GovernedLedgerError>;
}

pub(crate) struct FileGovernedExecutionLedger {
    // private root path
}
```

`reserve` atomically commits the nonce claim, authorization digest, execution identity, ownership
selector, trust snapshot, and initial envelope in one create-if-absent ledger record. Transitions
use head-digest compare-and-swap under a single-writer ledger actor. Every write syncs data and its
parent directory before success. Nonce tombstones remain beyond authorization expiry for a
configured safety window; clock skew is bounded and checked.

Reservation atomically rejects both a previously retained `(key_id, nonce)` and an existing
`execution_id`. Ledger access uses descriptor-relative no-follow opens, hashed safe filenames,
trusted ownership, owner-only modes, non-writable ancestors, post-open inode checks, and an
exclusive process lock for the ledger root.

### Service

```rust
pub(crate) struct RunControls {
    timeout: std::time::Duration,
    max_output_bytes: u64,
    ownership_selector: String,
}

pub struct StagedProcess {
    pub identity: ContainerProcessIdentity,
    // private non-cloneable release permit
}

#[async_trait]
pub trait ContainerRuntime {
    // existing methods unchanged

    async fn stage_process(
        &self,
        config: ContainerSpawnConfig,
    ) -> Result<StagedProcess, RuntimeError>;

    async fn release_process(
        &self,
        staged: StagedProcess,
    ) -> Result<ContainerProcess, RuntimeError>;
}

pub(crate) struct RunEngine {
    // focused image, lifecycle, event, policy, and state dependencies
}

pub(crate) struct PreparedRun {
    // sealed manifest, deterministic resource plans, and normalized request
}

pub(crate) struct AllocatedRun {
    // concrete owned resources and spawn configuration
}

pub(crate) struct StagedRun {
    // concrete process identity held behind a closed execution latch
}

pub(crate) struct RunningRun {
    // container, runtime, and process identities plus bounded output channels
}

impl RunEngine {
    pub(crate) async fn prepare(
        &self,
        request: NormalizedRunRequest,
        controls: &RunControls,
    ) -> Result<PreparedRun, RunEngineError>;

    pub(crate) async fn spawn(
        &self,
        allocated: AllocatedRun,
    ) -> Result<StagedRun, RunEngineError>;

    pub(crate) async fn allocate(
        &self,
        prepared: PreparedRun,
        controls: &RunControls,
    ) -> Result<AllocatedRun, RunEngineError>;

    pub(crate) async fn release(
        &self,
        staged: StagedRun,
    ) -> Result<RunningRun, RunEngineError>;

    pub(crate) async fn discover(
        &self,
        ownership_selector: &str,
    ) -> Result<Vec<GovernedResourceLocator>, RunEngineError>;

    pub(crate) async fn wait(
        &self,
        running: &mut RunningRun,
        controls: &RunControls,
    ) -> Result<RunOutcome, RunEngineError>;

    pub(crate) async fn cleanup(
        &self,
        ownership_selector: &str,
        locators: &[GovernedResourceLocator],
    ) -> Result<(), RunEngineError>;
}

#[async_trait]
pub(crate) trait GovernedExecutor: Send + Sync {
    async fn execute(
        &self,
        envelope: ExecutionEnvelopeV1,
        response_tx: tokio::sync::mpsc::Sender<DaemonResponse>,
    ) -> Result<(), GovernedExecutionError>;
}

pub(crate) struct GovernedExecutionService<V, L> {
    // private verifier, ledger, and focused run dependencies
}

impl<V, L> GovernedExecutionService<V, L>
where
    V: AuthorizationVerifier,
    L: GovernedExecutionLedger,
{
    pub(crate) async fn execute(
        &self,
        envelope: ExecutionEnvelopeV1,
        response_tx: tokio::sync::mpsc::Sender<DaemonResponse>,
    ) -> Result<(), GovernedExecutionError>;

    pub(crate) async fn reconcile_interrupted(&self) -> Result<u64, GovernedExecutionError>;
}

pub struct GovernedExecutorConfig {
    pub trust_store_path: std::path::PathBuf,
    pub audience: String,
    pub state_root: std::path::PathBuf,
    pub retention: GovernedRetentionConfig,
}

pub struct GovernedExecutorHandle {
    // opaque type-erased executor
}

impl GovernedExecutorHandle {
    pub fn build(
        config: GovernedExecutorConfig,
        dependencies: &HandlerDependencies,
    ) -> Result<Self, GovernedExecutorBuildError>;

    pub async fn reconcile_interrupted(&self) -> Result<u64, GovernedExecutionError>;
}

impl MiniboxServer {
    pub fn with_governed_executor(self, executor: GovernedExecutorHandle) -> Self;
}
```

Existing ordinary run handling delegates to `RunEngine` with compatibility controls. Governed
execution supplies signed finite controls and journals every locator. The extraction preserves
ordinary `Run` behavior and does not make the governed service depend on complete handler state.
The public handle and factory are the only governed implementation surface used by `miniboxd`;
verification, ledger, and service traits remain crate-private.

The staged runtime methods have a default `RuntimeError::UnsupportedCapability` implementation.
Only the native Linux adapter overrides them in V1. Conformance tests require every other adapter to
return unsupported without starting workload code.

## Configuration

The endpoint is disabled unless all settings are present:

```text
MINIBOX_AUTHORIZATION_TRUST_STORE=<absolute path>
MINIBOX_AUTHORIZATION_AUDIENCE=<unique daemon or deployment identity>
MINIBOX_GOVERNED_EXECUTION_ROOT=<absolute state path>
```

Invalid configuration never falls back to trusting the caller. Ordinary operations retain their
existing behavior. A local replay ledger supports a single-daemon audience only. Fleet-wide
audiences require a shared strongly consistent ledger and are out of scope.

## Data Flow

1. At the daemon raw frame boundary, reject frames over 1 MiB and duplicate JSON object keys before
   deserializing `DaemonRequest`; then parse the envelope and strict
   `(namespace = "minibox.container", name = "run.v1")` input.
2. Verify chain, decision, key, policy scope, audience, time window, digest, signature, and grant.
3. Resolve the already-cached image by the signed OCI manifest digest and reject a missing image or
   digest mismatch. Governed V1 never pulls a mutable tag as part of execution.
4. Derive and seal the exact workload manifest from input and the signed grant. The effective set
   must be contained by
   both requested and granted capabilities. Duplicate controls do not exist in the action input.
5. Apply `ContainerPolicy` and configured local `ExecutionPolicy` to the sealed manifest. Current
   local policies may deny
   but do not rewrite the signed workload. Append `Admission::Denied` or `Admission::Admitted` to
   the in-memory prefix; admitted records include environment evidence for the exact image and
   workload digest.
6. Atomically reserve the authorization nonce, admission-bearing envelope, trust snapshot, and
   deterministic ownership selector in one ledger transaction. Return after a committed denial;
   only an admitted reservation may continue.
7. Allocate resources from deterministic pre-journaled identities, require every resource to carry
   the ownership selector, and journal observed generations immediately.
8. Spawn behind a closed execution latch, durably record concrete process identity and generation,
   append `Started`, then release the latch.
9. Read output without retaining raw bytes, hash it incrementally, and terminate the workload when
   the finite signed output limit is exceeded. Append digest-only output evidence at completion.
10. Stop and clean up on exit, timeout, cancellation, disconnect, limit violation, or error.
11. Append exactly one result after cleanup evidence and return the completed envelope.

## Recovery

- V1 recovery is cleanup-only; it never adopts or resumes a prior workload.
- The reservation contains ownership selectors and every observed container, runtime, PID
  generation, rootfs, cgroup, and network locator needed for cleanup discovery.
- Interrupted records are retried with bounded backoff; exhausted cleanup enters an operator-visible
  quarantine state and blocks conflicting admission.
- Recovery never starts a second workload with the original nonce.
- Ledger or cleanup failures leave the execution nonterminal.

## Output And Evidence

- Raw stdout and stderr are not retained by the governed ledger in V1.
- Output is consumed through bounded buffers, hashed incrementally, and represented by digest,
  observed-byte count, forwarded-byte count, and truncation/limit status.
- The signed resource grant must contain finite wall-time, memory, process, and output limits.
- Native manifest evidence uses an owner-only local locator; output evidence has no locator.
- Per-execution and global ledger quotas plus TTL retention are configured in `miniboxd`.

## Trust And Time

- Authorization issue time may be at most 60 seconds in the future; expiry must be after issue time.
- A boot-scoped monotonic deadline is derived after verification so wall-clock rollback cannot extend
  authorization during an active execution.
- Replay tombstones remain for at least the maximum authorization lifetime plus the configured
  clock-skew window.

## Out Of Scope

- Signing inside Minibox or dynamic trust-store reload.
- Sigstore, SSH signatures, remote KMS, or secret decryption.
- Governed exec, build, pull, push, or workflow operations.
- Hostile isolation or environment-attestation claims.
- Changing existing `Run` behavior.
- Non-native adapter suites in governed V1.
- Fleet audiences without a shared strongly consistent replay ledger.
- Custom environment values, secret grants, network allowlists, unrestricted networking, and
  privileged execution in the first Minibox contract.

## Acceptance Criteria

- A golden Ed25519 authorization executes once and returns a valid envelope.
- Tampered payloads and replay fail before allocation.
- Replay remains denied after restart or failed execution.
- Local policy denial remains authoritative.
- Failure injection leaves recoverable evidence and no unowned resources.
- Trust-store ownership, mode, ancestor, symlink, authority, policy-digest, validity, and revocation
  tests pass.
- Property tests prove `effective <= grant <= request` and local policy never widens authority.
- Crash injection covers reservation, sync, allocation, spawn, output, cleanup, and finalization.
- Serializer and log tests prove plaintext secrets and custom environment values never persist.
- Protocol snapshots and property tests cover every new variant.
- Linux tests cover timeout, output bounds, cleanup, recovery, and replay.
- Suite-selection tests prove governed execution is available only on native Linux and fails closed
  on every other adapter suite.
