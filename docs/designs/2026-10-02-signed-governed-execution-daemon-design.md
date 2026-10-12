---
source_sha: 792ee4a567b12a6b78f5c84307e92a0e818fd2d4
status: approved
approved: 2026-10-02
sources:
  - crates/minibox-domain/src/execution_policy.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-domain/src/runtime.rs
  - crates/minibox-core/src/protocol.rs
  - crates/minibox/src/container/process.rs
  - crates/minibox/src/daemon/handler/run/model.rs
  - crates/minibox/src/daemon/handler/run/request.rs
  - crates/minibox/src/daemon/handler/run/preparation.rs
  - crates/minibox/src/daemon/handler/manifest.rs
  - crates/minibox/src/daemon/server.rs
  - crates/minibox/src/adapters/runtime.rs
  - crates/minibox-core/src/adapter_registry.rs
  - crates/minibox-crux-plugin/README.md
  - xtask/src/test_in_vm.rs
  - .ctx/godmode/plans/2026-09-22-signed-governed-execution-daemon.md
generated: 2026-10-02
---

# Design: Signed Governed Execution Daemon

## Goal

Enable Minibox to execute externally signed Crux envelopes on native Linux with strict
authorization verification, atomic replay and evidence reservation, bounded execution, and
cleanup-only recovery, while leaving ordinary `Run` behavior provably unchanged.

## Approved Approach

**Approach A — dual-path over a shared, extracted `RunEngine`.** Ordinary `Run` and governed
admission both drive one engine; governed wraps it with verification, reservation, and a staged
release latch.

Governance is scoped to the envelope schema and trust model, the authorization verifier, the
transactional ledger, admission, and the staged run engine. It does not replace the existing
`ExecutionPolicy` admission path, does not change `Run`, and does not alter the
`minibox-crux-plugin` stdio bridge.

## Crate Ownership

- **Owner crate**: `minibox-domain` — owns the envelope schema, trust-store schema, ledger
  record types, and the port traits. Already the home of `ExecutionPolicy`, `PolicyEvaluator`,
  and `ContainerRuntime`, and already depends on `sha2` and `slashcrux`, so pure-Rust crypto is
  in-category.
- **Affected crates**:
  - `minibox-domain` — new modules; `ContainerRuntime` gains two defaulted methods
  - `minibox-core` — additive protocol variants; re-exports the domain types unchanged
  - `minibox` — verifier, ledger, admission state machine, extracted engine, native adapter override
  - `miniboxd` — configuration, opaque handle factory, native-only gating
  - `minibox-testsuite` — adapter conformance suite for staged spawn
  - `xtask` — smolvm provisioning gate; harness fixes to the VM test runner

Dependency direction is unchanged: `minibox-domain` <- `minibox-core` <- `minibox` <-
`miniboxd`. No new crate is introduced.

## Public API

### Traits

```rust
// crates/minibox-domain/src/governed_authorization.rs
pub trait AuthorizationVerifier: Send + Sync {
    fn verify(&self, envelope: &GovernedEnvelope, now: SystemTime) -> Result<VerifiedGrant, AuthorizationError>;
    fn identifier(&self) -> &'static str;
}

// crates/minibox-domain/src/governed_ledger.rs
pub trait GovernedLedger: Send + Sync {
    fn reserve(&self, claim: &ReservationClaim) -> Result<LedgerRecord, LedgerError>;
    fn append(&self, record: &LedgerRecord, expected_head: Option<HeadDigest>) -> Result<HeadDigest, LedgerError>;
    fn load(&self, execution_id: &str) -> Result<Option<LedgerRecord>, LedgerError>;
    fn list_interrupted(&self) -> Result<Vec<LedgerRecord>, LedgerError>;
}

// crates/minibox-domain/src/runtime.rs (existing trait, extended)
pub trait ContainerRuntime: AsAny + Send + Sync {
    // existing methods unchanged, including:
    //   async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult>;
    //   async fn wait_for_exit(&self, runtime_id: Option<&str>, pid: u32) -> Result<i32>;

    /// Spawn a container process that remains parked before `execve` until
    /// `release` is called. Mirrors `spawn_process` so the two paths share a
    /// configuration type.
    async fn staged_spawn(
        &self,
        config: &ContainerSpawnConfig,
        identity: &ResourceIdentity,
    ) -> Result<StagedSpawnResult, StagedError> {
        Err(StagedError::Unsupported)
    }

    /// Release the latch, allowing the parked child to reach `execve`.
    async fn release(&self, staged: &mut StagedSpawnResult) -> Result<(), StagedError> {
        Err(StagedError::Unsupported)
    }
}
```

Both new methods are `async`, matching `spawn_process`, and reuse the existing
`ContainerSpawnConfig` rather than introducing a parallel configuration type. `StagedSpawnResult`
extends `SpawnResult` so the parked process is otherwise indistinguishable from an ordinary spawn.

`staged_spawn` and `release` carry default bodies returning `StagedError::Unsupported`. This is
load-bearing rather than a convenience: non-native adapters inherit the default and therefore fail
closed with no code written, which is the property task 8 must prove.

### Types

```rust
// crates/minibox-domain/src/governed_envelope.rs

pub const GOVERNED_ENVELOPE_SCHEMA_VERSION: u32 = 1;
pub const GOVERNED_SIGNING_CONTEXT: &str = "minibox.governed.envelope.v1";

/// What the envelope asks to run. Mirrors the fields a governed workload may
/// vary; anything not listed here cannot be influenced by a signed request.
pub struct GovernedRequest {
    pub image_ref: String,
    pub image_manifest_digest: String,
    pub command: Vec<String>,
    pub entrypoint: Option<Vec<String>>,
    pub network_mode: NetworkMode,
    pub privileged: bool,
    pub mounts: Vec<GovernedMount>,
    pub resource_limits: ResourceLimits,
    pub platform: Option<String>,
}

pub struct GovernedMount {
    pub host_path: String,
    pub container_path: String,
    pub read_only: bool,
}

pub struct ResourceLimits {
    /// Bounded to 2^53 - 1 so the value survives JCS IEEE 754 double serialization.
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u32>,
    /// Bounded to 2^53 - 1 so the value survives JCS IEEE 754 double serialization.
    pub pids_limit: Option<u64>,
}

/// Capabilities a grant may authorize. Deny-by-default: a capability absent
/// from this set is never available to the workload.
pub struct CapabilitySet {
    pub allow_custom_environment: bool,
    pub allow_secrets: bool,
    pub allow_network: bool,
    pub allow_privilege: bool,
}

pub struct GovernedEnvelope {
    pub schema_version: u32,
    pub key_id: String,
    // Encoded as RFC 3339 UTC strings with exactly millisecond precision.
    pub issued_at: SystemTime,
    pub not_before: SystemTime,
    pub not_after: SystemTime,
    pub nonce: [u8; 32],
    pub execution_id: String,
    pub grant: Grant,
    pub request: GovernedRequest,
    pub signature: Signature,
}

pub struct Grant {
    pub audience: String,
    pub subject: String,
    pub scope: AuthorityScope,
    pub policy_digest: [u8; 32],
    pub resource_ceiling: ResourceLimits,
    pub capabilities: CapabilitySet,
}

pub struct VerifiedGrant {
    pub key_id: String,
    pub subject: String,
    pub scope: AuthorityScope,
    pub effective: GovernedRequest,
}

pub struct AuthorityScope {
    pub permitted_intents: Vec<String>,
    pub permitted_images: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; 64]);

pub struct VerifyingKey([u8; 32]);

pub enum SigningError { Canonicalization, Encoding, KeyGeneration, Verification }
pub enum CanonicalizationError {
    UnsupportedVersion,
    NonFiniteNumber,
    InvalidUtf8,
    DuplicateProperty,
    IntegerOutOfRange,
    TimestampPrecision,
}

// crates/minibox-domain/src/trust_store.rs

pub struct TrustStoreEntry {
    pub id: String,
    pub public_key: VerifyingKey,
    pub scope: AuthorityScope,
    pub policy_digest: [u8; 32],
    // Encoded as RFC 3339 UTC strings with exactly millisecond precision.
    pub not_before: SystemTime,
    pub not_after: SystemTime,
    pub revoked: bool,
}

pub enum TrustStoreError {
    NotRegularFile,
    SymlinkRejected,
    WrongOwner,
    GroupOrWorldWritable,
    WritableAncestor,
    IdMismatch,
    DuplicateId,
    AliasedKey,
    MalformedEntry,
}

pub struct TrustStoreSnapshot {
    entries: Vec<TrustStoreEntry>,
}

impl TrustStoreSnapshot {
    pub fn lookup(&self, key_id: &str) -> Option<&TrustStoreEntry>;
}

// crates/minibox-domain/src/governed_ledger.rs

pub enum ExecutionPhase {
    Reserved,
    Preparing,
    Allocated,
    Spawned,
    Started,
    Running,
    Finalizing,
    Finalized,
    CleanedUp,
    Failed,
}

pub struct OwnershipSelector {
    pub execution_id: String,
    pub planned_resources: Vec<ResourceLocator>,
}

pub struct ResourceLocator {
    pub kind: ResourceKind,
    pub selector: String,
}

pub enum ResourceKind { Overlay, Cgroup, Socket, StagedProcess }

pub struct ReservationClaim {
    pub execution_id: String,
    pub key_id: String,
    pub nonce: [u8; 32],
    pub envelope: GovernedEnvelope,
    pub trust_snapshot_digest: [u8; 32],
    pub resource_plan: OwnershipSelector,
}

pub struct LedgerRecord {
    pub execution_id: String,
    pub phase: ExecutionPhase,
    pub claim: ReservationClaim,
    pub observed_locators: Vec<ResourceLocator>,
    pub terminal_envelope: Option<GovernedEnvelope>,
    pub head: HeadDigest,
}

pub struct HeadDigest([u8; 32]);

pub enum LedgerError {
    DuplicateNonce,
    DuplicateExecution,
    CasMismatch,
    NotFound,
    QuotaExceeded,
    LockUnavailable,
    Io,
}

pub enum AuthorizationError {
    UnknownKey,
    RevokedKey,
    OutsideValidityWindow,
    SignatureInvalid,
    AudienceMismatch,
    ScopeInsufficient,
    PolicyDigestMismatch,
    ClockSkewExceeded,
}

// crates/minibox-domain/src/runtime.rs (existing trait, new supporting types)

/// Identity derived before any mutation, so a crash mid-mutation leaves
/// resources discoverable by derivation rather than by search.
pub struct ResourceIdentity {
    pub execution_id: String,
    pub resource_kind: ResourceKind,
    pub tag: String,
}

/// Result of a staged spawn. Extends `SpawnResult` with the parked latch so
/// the caller can observe "spawned but not released" without racing.
pub struct StagedSpawnResult {
    pub spawn: SpawnResult,
    pub identity: ResourceIdentity,
    pub released: bool,
}

pub enum StagedError { Unsupported, SpawnFailed, IdentityNotDurable }
```

### Functions

```rust
// crates/minibox-domain/src/governed_envelope.rs

/// Produce the exact bytes covered by `signature`.
///
/// Implements RFC 8785 (JCS) over the field set enumerated in "Canonicalization",
/// prefixed with `GOVERNED_SIGNING_CONTEXT`. The `signature` field is excluded
/// by construction.
pub fn canonical_signing_bytes(envelope: &GovernedEnvelope) -> Result<Vec<u8>, CanonicalizationError>;

/// Structural validation only: known schema version, required fields present.
pub fn validate_structure(bytes: &[u8]) -> Result<(), CanonicalizationError>;

/// Field-wise pointwise minimum. Where fields disagree the lower value wins;
/// no other precedence rule exists.
pub fn narrow_effective(
    request: &GovernedRequest,
    grant: &Grant,
    ceiling: &GovernedRequest,
) -> GovernedRequest;
```

```rust
// crates/minibox/src/daemon/governed/trust_store.rs

/// Load and validate every entry under `dir` into an immutable snapshot.
pub fn load_trust_store(dir: &Path) -> Result<TrustStoreSnapshot, TrustStoreError>;
```

Signature verification is `Signature::verify` against `VerifyingKey` using `ed25519_dalek`,
implemented in `minibox::daemon::governed::authorization` rather than the domain.

Verification MUST use `VerifyingKey::verify_strict`. Plain `verify()` permits weak public keys —
low-order points for which a signature verifies against almost any message — which would leave a
forgery path in the one place this design exists to close. The `legacy_compatibility` feature MUST
NOT be enabled; it reinstates signature malleability.

## Canonicalization

The signed bytes are produced by RFC 8785 JSON Canonicalization Scheme, prefixed with the fixed
context string `minibox.governed.envelope.v1`.

RFC 8785 mandates the following, and this design adopts each as normative rather than
reimplementing it:

- **Recursive property sorting** (§3.2.3). Object properties are sorted by **UTF-16 code unit**
  value, comparing code units as unsigned integers independent of locale. Array element order is
  **not** changed.
- **ECMAScript primitive serialization** (§3.2.2). Strings, numbers, and literals serialize per
  ECMAScript; the result is then encoded as UTF-8 (§3.2.4). No whitespace between tokens (§3.2.1).
- **I-JSON constraints** (§3.1). JSON objects **must not** exhibit duplicate property names;
  numbers must be expressible as IEEE 754 double-precision values; strings must be valid Unicode.
  NaN and Infinity must cause the implementation to error rather than serialize.

Four rules are load-bearing on top of the spec:

1. **The sort is reimplemented, not inherited.** `serde_json`'s map representation is not part of
   the contract. A change to its `preserve_order` setting must not change any signed byte. The
   UTF-16 code-unit sort is implemented explicitly and independently of serde's iteration order.
2. **Explicit field set, signature excluded by construction.** The envelope's signed field set is
   a declared list. The `signature` field is excluded structurally rather than by filtering after
   the fact, so it cannot be serialized into the bytes it covers.
3. **Domain separation.** The context prefix is prepended before signing, so a signature over an
   envelope cannot be replayed as a signature over any other JSON structure the daemon accepts.
4. **Integer and timestamp encodings are pinned** (see below), because serde's defaults would
   otherwise make canonical bytes depend on a dependency's internals.

### Pinned encodings

Two field classes cannot use serde's default representation without making canonical bytes depend
on implementation detail:

- **64-bit integers.** JCS numbers are IEEE 754 doubles. `memory_limit_bytes` and `pids_limit` are
  bounded to `2^53 - 1` and rejected above it, with `CanonicalizationError::IntegerOutOfRange`.
  RFC 8785 Appendix D recommends encoding such numbers as JSON strings; a bounded integer is
  preferred here because a string would be silently mistyped by a non-conforming client.
- **Timestamps.** `SystemTime` has no canonical serde form — serde emits a struct, so canonical
  bytes would depend on serde's impl rather than on this spec. Timestamps are encoded as RFC 3339
  UTC strings with exactly millisecond precision. Second- and nanosecond-precision values are
  rejected as `CanonicalizationError::TimestampPrecision` so no client can lose precision
  silently across the signature boundary.

RFC 8785's published test vectors are vendored as golden fixtures. Signature verification covers
the canonical bytes directly; no pre-hash step is introduced.

## Data Flow

1. **Source** — raw client frame arrives on the Unix socket.
2. **Frame validation** — enforce the 1 MiB limit and reject duplicate JSON keys at every
   nesting depth, before any typed deserialization. This is required twice over: it is plan task 3,
   and RFC 8785 I-JSON conformance independently mandates that objects carry no duplicate
   property names.
3. **Structural parse** — `validate_structure`, then deserialize into `GovernedEnvelope`.
4. **Canonicalize** — `canonical_signing_bytes` yields the bytes the signature covers.
5. **Signature verify** — the first operation that trusts any envelope field.
6. **Trust resolution** — `key_id` resolves in the loaded snapshot; not revoked; within validity.
7. **Temporal and audience checks** — `not_before <= now <= not_after` under bounded skew;
   `grant.audience` matches this daemon.
8. **Scope and policy digest** — `grant.scope` covers `request`; request policy matches
   `grant.policy_digest`.
9. **Local ceiling** — build an `ExecutionManifest` from the candidate effective request and
   evaluate it through the existing `PolicyEvaluator`; the result may only lower.
10. **Effective request** — `effective = min(request, grant, local_ceiling)` field by field.
11. **Reserve** — append a `Reserved` record, consuming the nonce before any allocation.
12. **Engine** — drive `RunEngine` through normalize, prepare, allocate, staged spawn.
13. **Staged release** — journal PID identity and `Started` durably, then call `release`.
14. **Wait, discover, finalize** — bounded wait, resource discovery, single terminal CAS.
15. **Cleanup** — idempotent teardown from any non-terminal phase.
16. **Sink** — terminal envelope appended by CAS and returned on the response stream.

Step 9 is reuse, not new code: `ExecutionPolicy` already denies on network mode, privileged flag,
memory ceiling, and mount prefixes, and any future `PolicyEvaluator` implementation becomes a
governed ceiling without touching the admission path.

## Validation Ordering Invariant

Steps 5 through 10 are strictly sequential and fail closed. No operation may branch on envelope
contents before step 5 completes — not trust-store lookup, not size-dependent allocation, and not
error text that distinguishes an unknown key from an invalid signature.

This is enforced by test: a well-formed but tampered envelope must be observationally
indistinguishable from random bytes.

## Phases and Concurrency

```text
Reserved -> Preparing -> Allocated -> Spawned -> Started -> Running -> Finalizing -> Finalized
                     \______________________________________________________________/
                                        \-> CleanedUp (from any non-terminal phase)
```

Each transition is a CAS append against `(execution_id, expected_head)` over a hash chain, so
competing writers are detected rather than silently interleaved and later tampering is visible.

The ledger is single-process. Transitions are serialized in-process behind an async mutex; the
hash-chain CAS still runs and still detects logic errors. A second daemon attempting to use the
same data directory is rejected at startup rather than defended with per-transition locking.

Durability per transition: write temp file, `fsync` the file, atomic `rename`, `fsync` the parent
directory. Crash-injection points hook exactly these boundaries.

`OwnershipSelector` records what the ledger intended to create; `observed_locators` records what
was observed after each mutation. A discrepancy discovered during recovery means an unowned
resource exists and triggers quarantine rather than silent cleanup.

## Staged Release

`crates/minibox/src/container/process.rs` already establishes a parent/child barrier: the parent
creates a `pipe2(O_CLOEXEC)`, forgets both `OwnedFd`s for manual lifetime control, calls
`clone_with_namespaces`, and the child blocks on `read` until the parent writes one byte. That
barrier currently gates `uid_map` installation.

Governance reuses the same barrier and extends the release condition:

```text
ordinary Run:  clone -> child blocks -> parent writes (after uid_map)         -> execve
governed:      clone -> child blocks -> parent writes -> execve
                                 only after PID identity and `Started` are journaled durable
```

The child is parked after `clone` and before `execve`, so the PID exists and is journalable while
the workload provably has not started. Ordinary `Run` retains its current timing, so the
characterization guarantee holds by construction rather than by assertion.

The daemon owns the release moment through the explicit `StagedSpawnResult` handle. This keeps the ordering
invariant in the daemon and lets a test observe "spawned but not released" synchronously, with no
sleeping or racing, which is what makes the crash-injection campaigns deterministic.

## Recovery

At startup, scan for records in non-terminal phases, resolve ownership through the selector and
observed locators, clean up idempotently, and finalize exactly once. Idempotency is what makes
cleanup safe to retry after a crash partway through it.

## Hexagonal Boundaries

- **Port**: `AuthorizationVerifier` in `minibox_domain::governed_authorization`
- **Adapter**: `Ed25519AuthorizationVerifier` in `minibox::daemon::governed::authorization`
- **Port**: `GovernedLedger` in `minibox_domain::governed_ledger`
- **Adapter**: `FileGovernedLedger` in `minibox::daemon::governed::store`
- **Port**: `ContainerRuntime::staged_spawn` / `release` in `minibox_domain::runtime`
- **Adapter**: `LinuxNamespaceRuntime` override in `minibox::adapters::runtime`
- **Trust store load**: `load_trust_store(dir: &Path) -> Result<TrustStoreSnapshot, TrustStoreError>`
  in `minibox::daemon::governed::trust_store` — not a trait, because no alternative backend exists
  and YAGNI applies
- **Policy ceiling**: existing `PolicyEvaluator` port, unchanged

The ledger does not reuse `StateStore`. `StateStore` is a general container-state port; the
governed ledger needs CAS-on-head, tombstones, TTL pruning, and quota accounting that would
distort that trait if forced into it.

## Trust Store

One JSON file per key at `<trust_dir>/<key_id>.json`. A directory rather than a single document
because the required checks — non-regular file, symlink, duplicate id, aliased key across files —
are only meaningful in that shape, and because revoking a key is then a single atomic file
operation instead of a read-modify-write of one document.

Load validates: no symlinks (`O_NOFOLLOW`), regular files only, owned by the daemon uid, not
group- or world-writable, no writable ancestor directory, `entry.id` equal to the filename stem,
and rejects duplicate ids or duplicate public keys across files.

Loaded once at startup into an immutable `TrustStoreSnapshot`, replaced by atomic swap on explicit
reload. Verification does not re-read the filesystem per request.

## Out of Scope

- Key delegation between trusted keys; deferred behind a `schema_version` bump
- mTLS or any transport authentication; `minibox-crux-plugin` remains an explicitly
  unauthenticated local stdio bridge
- Two daemons sharing one data directory; rejected at startup
- Any change to ordinary `Run` semantics
- Ceiling precedence beyond pointwise `min()`
- Key rotation beyond add and revoke via the trust store
- Replacing or deprecating the existing `ExecutionPolicy` admission path

## Risk

- [ ] **Breaking API changes** — yes, two additive-only:
  - `ContainerRuntime` gains `staged_spawn` and `release` with default bodies. Source-compatible;
    all 13 implementors across 3 crates (`minibox-core`, `minibox`, `macbox`) compile unchanged.
  - `DaemonRequest` gains `GovernedRun`; `DaemonResponse` gains governed output and terminal
    variants. Additive. Existing fixtures must continue to pass.
- [ ] **Terminal-response logic** — `DaemonResponse::ContainerOutput` is non-terminal while most
      other variants end request streaming. Adding a governed streaming variant requires auditing every
      terminal-response decision in the dispatch path, or governed requests will hang or truncate. This
      is in scope for the protocol task, not deferred.
- [ ] **New external dependency** — yes, `ed25519-dalek`, plus the RFC 8785 vectors vendored as
      test fixtures. Justified: Ed25519 is the wire format, so the signature type is concrete rather
      than abstracted behind a trait; verification logic remains behind `AuthorizationVerifier`.
  - Version: `3.x`, whose MSRV is **1.98 — exactly this workspace's `rust-version`**
    (`Cargo.toml:30`). 2.x requires only 1.60. If 3.x is still pre-release at implementation time,
    pin 2.x explicitly and re-check `verify_strict` availability.
  - `legacy_compatibility` MUST NOT be enabled; it reinstates signature malleability.
  - Consequence to accept: `ed25519-dalek` becomes a transitive dependency of `minibox-core`.
    In-category with the `sha2` dependency `minibox-domain` already carries.
- [ ] **Serialization format change** — none to existing formats. New ledger files are a new
      on-disk artifact under daemon-owned state.
- [ ] **Design rule violation** — this design spans 6 crates against the skill's 3-crate guidance.
      Accepted as a consequence of scoping all 10 plan tasks to a single design.
- [ ] **Verification gap** — the envelope, trust store, ledger model, and `effective <= grant <=
request` properties are verifiable on any host. Adapter conformance, staged-release durability,
      and the crash-injection campaigns require the smolvm Linux environment, which is not currently
      provisioned.
