---
source_sha: f5481a9482fbb04690db6b7a52ee8eca9c5fe5e9
status: superseded
superseded_by: docs/designs/2026-09-18-environment-attested-isolation-design.md
sources:
  - crates/minibox-domain/src/lib.rs
  - crates/minibox-domain/src/runtime.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-domain/src/execution_policy.rs
  - crates/minibox/src/container/process.rs
  - crates/minibox/src/container/namespace.rs
  - crates/minibox/src/container/mount_seccomp.rs
  - crates/minibox/src/adapters/runtime.rs
  - crates/minibox/src/adapters/smolvm.rs
  - crates/minibox/src/adapters/colima.rs
  - crates/minibox/src/adapters/gke.rs
  - crates/minibox/src/adapters/hcs.rs
  - crates/minibox/src/adapters/wsl2.rs
  - crates/minibox-testsuite/src/adapters/runtime.rs
  - docs/designs/2026-09-12-guaranteed-security-profiles-design.md
generated: 2026-09-18
---

# Design: Typed Isolation Policy Contract

> Superseded by `docs/designs/2026-09-18-environment-attested-isolation-design.md`.

## Goal

Establish versioned, adapter-independent isolation profiles that every qualified runtime must
enforce, explicitly degrade, or reject before Minibox claims a successful container launch.

## Approved Approach

Use a domain-owned typed isolation policy compiled by each runtime into equivalent native or VM
mechanisms, with explicit optional degradation only in the compatibility profile.

## Relationship To Guaranteed Security Profiles

This is the Phase-0 bridge into
`docs/designs/2026-09-12-guaranteed-security-profiles-design.md`. It establishes the portable
control vocabulary, adapter planning contract, evidence shape, and conformance suite without
claiming hostile multi-tenant qualification.

The later signed design has richer, parameterized `ControlRequirement` values. It replaces
`ResolvedIsolationPolicy` and `IsolationRuntime` as the execution authority rather than translating
signed controls into this coarser enum. Phase-0 control IDs, mechanism labels, negative tests, and
cleanup behavior may be reused, but they are not a lossless signed-policy representation.

Phase-0 evidence is structural and local, not cryptographically authenticated. Signing, replay
prevention, trust stores, custom bundles, and remote attestation remain in the later design. Hostile
profiles never use Phase-0 compatibility degradation; they enter only after the signed compiler has
replaced the Phase-0 authority.

## Context Map

### Files To Modify

| File                                                 | Purpose                         | Change                                                                               |
| ---------------------------------------------------- | ------------------------------- | ------------------------------------------------------------------------------------ |
| `crates/minibox-domain/src/isolation_policy.rs`      | New portable contract           | Add profile, control, resolution, plan, evidence, and error types                    |
| `crates/minibox-domain/src/lib.rs`                   | Domain exports                  | Export the isolation policy module                                                   |
| `crates/minibox-domain/src/isolation_runtime.rs`     | New secure launch port          | Add plan, launch, stop, and cleanup operations without changing the raw runtime port |
| `crates/minibox-domain/src/execution_manifest.rs`    | Persisted evidence              | Add an optional versioned isolation record                                           |
| `crates/minibox/src/daemon/isolation.rs`             | Application service             | Resolve policy and validate plan/evidence transitions                                |
| `crates/minibox/src/container/process.rs`            | Native enforcement              | Apply the native plan before `execve` and return evidence                            |
| `crates/minibox/src/adapters/isolation.rs`           | Secure runtime wrapper          | Own preparation, raw runtime, rollback, and evidence for qualified minibox adapters  |
| `crates/minibox-testsuite/src/adapters/isolation.rs` | Shared conformance              | Verify semantic behavior across adapters                                             |
| `crates/minibox-core/tests/proptest_roundtrip.rs`    | Manifest compatibility consumer | Add the optional isolation field to manifest generators                              |
| `crates/minibox-core/tests/property_roundtrip.rs`    | Manifest compatibility consumer | Round-trip old and isolated manifest shapes                                          |

### Dependencies And Consumers

| File                                             | Relationship                                                                      |
| ------------------------------------------------ | --------------------------------------------------------------------------------- |
| `crates/minibox/src/daemon/handler/run.rs`       | A later admission slice replaces raw preparation with the secure launch port      |
| `crates/minibox/src/daemon/handler/run/model.rs` | A later admission slice carries the resolved policy into secure launch            |
| `crates/macbox/src/krun/runtime.rs`              | Remains source-compatible and unqualified until a separate adapter slice wraps it |
| `crates/macbox/src/vz/adapter.rs`                | Remains source-compatible and unqualified until a separate adapter slice wraps it |

The macOS and Windows crates are consumers of the unchanged raw runtime contract but are not
implementation targets in this contract slice. Their secure wrappers require subsequent
adapter-qualification designs.

### Existing Test Coverage

| Test                                                     | Current Coverage                      | Required Extension                                               |
| -------------------------------------------------------- | ------------------------------------- | ---------------------------------------------------------------- |
| `crates/minibox-domain/src/runtime.rs`                   | Runtime capability shape              | Profile ordering, monotonic resolution, plan/evidence invariants |
| `crates/minibox/tests/native_adapter_isolation_tests.rs` | Namespaces, cgroups, overlay behavior | Strict and compatibility policy behavior                         |
| `crates/minibox/tests/security_regression.rs`            | Existing host escape regressions      | Capability, `no_new_privs`, seccomp, and degradation records     |
| `crates/minibox-testsuite/src/adapters/runtime.rs`       | Generic runtime spawn contract        | Required rejection and equivalent-control evidence               |

### Reference Patterns

| File                                               | Pattern                                              |
| -------------------------------------------------- | ---------------------------------------------------- |
| `crates/minibox-domain/src/runtime.rs`             | Domain-owned runtime port and capability report      |
| `crates/minibox-domain/src/execution_manifest.rs`  | Additive persisted schema and deterministic evidence |
| `crates/minibox/src/container/mount_seccomp.rs`    | Pure plan construction plus real-kernel verification |
| `crates/minibox-testsuite/src/adapters/runtime.rs` | Shared adapter conformance registration              |

### Risk

- The existing `ContainerRuntime`, `ContainerSpawnConfig`, and `SpawnResult` remain source-compatible.
- `ExecutionManifest` changes additively and retains old-manifest deserialization.
- No new crate or external dependency is introduced.
- Phase 0 does not qualify any adapter for hostile multi-tenancy.

## Crate Ownership

- **Contract owner: `minibox-domain`** - owns portable controls, built-in profile definitions,
  resolution rules, plans, evidence, and typed errors.
- **Application and adapter owner: `minibox`** - resolves policies at the run boundary and maps them
  to native or adapter-specific mechanisms.
- **Conformance owner: `minibox-testsuite`** - owns the semantic contract shared by every qualified
  `IsolationRuntime`.

The primary ownership remains in these three crates. The approved design review explicitly waives
the three-crate limit so `minibox-core` compatibility fixtures can update in the same change as the
additive manifest field. Daemon configuration and wire selection remain in the dependent admission
design.

## Built-In Profiles

Profile identifiers are stable protocol values:

| Profile            | Required Controls                                                                                                               | Optional Controls                                                                                     |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `strict-v1`        | All controls                                                                                                                    | None                                                                                                  |
| `standard-v1`      | Process boundary, mount boundary, network boundary, capability ceiling, `no_new_privs`, syscall filter, filesystem restrictions | None                                                                                                  |
| `compatibility-v1` | Process boundary, mount boundary, capability ceiling, `no_new_privs`                                                            | User identity, network boundary, syscall filter, filesystem restrictions, memory limit, process limit |

Only `compatibility-v1` may produce degraded controls. A request may promote any omitted or optional
control to required, but cannot remove a profile requirement.

## Public API

All types below live in `minibox_domain::isolation_policy` unless noted otherwise.

### Profile And Control Types

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationProfileId {
    CompatibilityV1,
    StandardV1,
    StrictV1,
}

impl IsolationProfileId {
    pub const fn as_str(self) -> &'static str;
    pub const fn strength(self) -> u8;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationControl {
    UserIdentity,
    ProcessBoundary,
    MountBoundary,
    NetworkBoundary,
    CapabilityCeiling,
    NoNewPrivileges,
    SyscallFilter,
    FilesystemRestrictions,
    MemoryLimit,
    ProcessLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationProfile {
    pub id: IsolationProfileId,
    pub required_controls: BTreeSet<IsolationControl>,
    pub optional_controls: BTreeSet<IsolationControl>,
}

pub fn built_in_isolation_profile(id: IsolationProfileId) -> IsolationProfile;
```

Arbitrary profile construction is not a public execution surface. Callers select a built-in ID and
may add requirements through `IsolationProfileRequest`.
`strength()` returns `0` for compatibility, `1` for standard, and `2` for strict; declaration order
and derived ordering match those values.

### Resolution Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationPolicyCeiling {
    pub minimum_profile: IsolationProfileId,
    pub allowed_profiles: BTreeSet<IsolationProfileId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationProfileRequest {
    pub profile: IsolationProfileId,
    pub additional_required_controls: BTreeSet<IsolationControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedIsolationPolicy {
    pub minimum_profile: IsolationProfileId,
    pub requested_profile: IsolationProfileId,
    pub required_controls: BTreeSet<IsolationControl>,
    pub optional_controls: BTreeSet<IsolationControl>,
}

pub fn resolve_isolation_policy(
    ceiling: &IsolationPolicyCeiling,
    request: &IsolationProfileRequest,
) -> Result<ResolvedIsolationPolicy, IsolationPolicyError>;
```

Resolution rejects disallowed profiles and requests weaker than the daemon minimum. The effective
required set is the union of the daemon minimum, selected profile requirements, and additional
requirements. Optional controls never override a required control.

### Adapter Planning Types

```rust
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationEvidenceKind {
    LocalKernel,
    GuestAgent,
    Hypervisor,
    OuterSandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationCapabilities {
    pub adapter: String,
    pub supported_controls: BTreeSet<IsolationControl>,
    pub evidence_kinds: BTreeSet<IsolationEvidenceKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedIsolationControl {
    pub control: IsolationControl,
    pub mechanism: String,
    pub evidence_kind: IsolationEvidenceKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DegradedIsolationControl {
    pub control: IsolationControl,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationPlan {
    pub schema_version: u32,
    pub adapter: String,
    pub policy: ResolvedIsolationPolicy,
    pub applied_controls: Vec<PlannedIsolationControl>,
    pub degraded_controls: Vec<DegradedIsolationControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedIsolationEvidence {
    pub control: IsolationControl,
    pub mechanism: String,
    pub evidence_kind: IsolationEvidenceKind,
    pub evidence_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationEvidence {
    pub schema_version: u32,
    pub adapter: String,
    pub profile: IsolationProfileId,
    pub applied_controls: Vec<AppliedIsolationEvidence>,
    pub degraded_controls: Vec<DegradedIsolationControl>,
}
```

Mechanism names are adapter-owned stable labels used for audit and conformance, not executable
commands or caller-supplied payloads.

### Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum IsolationPolicyError {
    #[error("invalid isolation policy ceiling: {reason}")]
    InvalidCeiling { reason: String },
    #[error("unknown isolation profile {name}")]
    UnknownProfile { name: String },
    #[error("isolation profile {profile:?} is not allowed")]
    ProfileNotAllowed { profile: IsolationProfileId },
    #[error("isolation profile {requested:?} is weaker than daemon minimum {minimum:?}")]
    WeakerThanMinimum {
        requested: IsolationProfileId,
        minimum: IsolationProfileId,
    },
    #[error("adapter {adapter} does not support required isolation control {control:?}")]
    UnsupportedRequiredControl {
        adapter: String,
        control: IsolationControl,
    },
    #[error("adapter {adapter} returned invalid isolation plan: {reason}")]
    InvalidPlan { adapter: String, reason: String },
    #[error("adapter {adapter} returned invalid isolation evidence: {reason}")]
    InvalidEvidence { adapter: String, reason: String },
    #[error("adapter {adapter} failed isolation enforcement: {reason}")]
    EnforcementFailed { adapter: String, reason: String },
}
```

### Secure Isolation Runtime Port

```rust
#[async_trait]
pub trait IsolationRuntime: AsAny + Send + Sync {
    async fn isolation_capabilities(
        &self,
    ) -> Result<IsolationCapabilities, IsolationPolicyError>;

    async fn plan_isolation(
        &self,
        policy: &ResolvedIsolationPolicy,
    ) -> Result<IsolationPlan, IsolationPolicyError>;

    async fn spawn_isolated(
        &self,
        request: &IsolationLaunchRequest,
        plan: &IsolationPlan,
    ) -> Result<IsolatedSpawnResult, IsolationPolicyError>;

    async fn wait_for_exit(&self, runtime_id: Option<&str>, pid: u32) -> Result<i32>;

    async fn stop(&self, container_id: &str) -> Result<(), IsolationPolicyError>;

    async fn cleanup(&self, container_id: &str) -> Result<(), IsolationPolicyError>;
}

pub type DynIsolationRuntime = Arc<dyn IsolationRuntime>;

#[derive(Debug, Clone)]
pub struct IsolationLaunchRequest {
    pub container_id: String,
    pub image_layers: Vec<InternalPath>,
    pub container_dir: InternalPath,
    pub run_dir: InternalPath,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub hostname: String,
    pub capture_output: bool,
    pub hooks: ContainerHooks,
    pub mounts: Vec<BindMount>,
    pub privileged: bool,
    pub image_ref: Option<String>,
    pub resources: ResourceConfig,
    pub network: NetworkMode,
}

pub struct IsolatedSpawnResult {
    pub spawn: SpawnResult,
    pub evidence: IsolationEvidence,
    pub rootfs: RootfsLayout,
    pub cgroup_path: InternalPath,
    pub network_allocation: Option<String>,
}
```

`IsolationRuntime` is the sole Phase-0 planning and enforcement authority. Its implementation owns
the filesystem, limiter, network, and raw `ContainerRuntime` adapters needed for launch and rollback.
The raw runtime port remains source-compatible and is not injected into isolated admission paths.
A successful spawn without valid evidence is converted into `InvalidEvidence`, followed by `stop`
and `cleanup`.

### Execution Manifest Extension

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionIsolationStatus {
    Planned,
    Enforced,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionIsolationRecord {
    pub status: ExecutionIsolationStatus,
    pub policy: ResolvedIsolationPolicy,
    pub plan: IsolationPlan,
    pub evidence: Option<IsolationEvidence>,
}
```

| Existing Type       | Exact Field Added                                                                                            |
| ------------------- | ------------------------------------------------------------------------------------------------------------ |
| `ExecutionManifest` | `#[serde(default, skip_serializing_if = "Option::is_none")] pub isolation: Option<ExecutionIsolationRecord>` |

Old manifests remain readable. New runs persist `Planned` before spawn, then atomically replace it
with `Enforced` and matching evidence or `Failed` without evidence after cleanup.

## Data Flow

1. The application resolves the daemon ceiling and request into `ResolvedIsolationPolicy`.
2. The selected isolation runtime probes semantic capabilities and compiles the policy into
   `IsolationPlan`.
3. Unsupported required controls reject before allocation; unsupported compatibility controls enter
   `degraded_controls`.
4. The application persists the planned manifest and passes the exact plan into
   `IsolationRuntime::spawn_isolated`.
5. The isolation runtime allocates resources, applies native or equivalent VM controls, and returns
   `IsolationEvidence`.
6. The application validates plan/evidence equality and finalizes the manifest.
7. Any allocation or enforcement failure triggers cleanup and a `Failed` manifest record.

## Hexagonal Boundaries

- **Domain contract:** `minibox-domain::isolation_policy` defines semantics without platform I/O.
- **Raw runtime port:** `ContainerRuntime` remains a low-level, source-compatible adapter primitive.
- **Secure launch port:** `IsolationRuntime` is the only Phase-0 profile planner and enforcer exposed
  to isolated admission.
- **Adapters:** implementations in `minibox` map controls to Linux, VM, or outer-sandbox mechanisms.
- **Application service:** `minibox::daemon::isolation` coordinates resolution and evidence checks
  without performing syscalls.

## Adapter Semantics

- Native Linux uses namespaces, capability sets, `PR_SET_NO_NEW_PRIVS`, seccomp, mount policy, and
  cgroups.
- VM runtimes may satisfy process, mount, network, and identity controls through a hypervisor and
  guest agent rather than host Linux syscalls.
- Managed runtimes may use verified outer-sandbox controls.
- An adapter that cannot map a required semantic control returns `UnsupportedRequiredControl`.
- An adapter never reports an equivalent control without naming the mechanism and evidence kind.

## Verification Strategy

- Property tests prove profile ordering and monotonicity: daemon baselines cannot be weakened.
- Table tests pin every built-in profile and stable serialized identifier.
- Shared conformance tests require either complete evidence or pre-allocation rejection.
- Compatibility tests prove every unsupported optional control is recorded exactly once.
- Failure injection at plan, allocation, enforcement, evidence validation, and persistence verifies
  transactional cleanup.
- Real Linux tests verify native controls; VM qualification tests verify equivalent outcomes.

## Out Of Scope

- Caller-authored profiles or raw seccomp, capability, VM, or HCS payloads.
- Signed authorization, trust roots, replay protection, and cryptographic evidence.
- Rootless support claims.
- Privileged launcher helpers.
- Hostile multi-tenant qualification before the later signed-profile design lands.

## Compatibility And Risk

- `ExecutionManifest.isolation` is wire-additive for historical reads.
- `ContainerRuntime`, `ContainerSpawnConfig`, and `SpawnResult` remain source-compatible.
- Qualified `minibox` isolation wrappers must land with conformance updates; unwrapped adapters remain
  explicitly unqualified rather than receiving permissive defaults.
- `minibox-core` manifest generators and round-trip fixtures update for the additive field.
- No external dependency or feature flag is required.

## Acceptance Criteria

1. Built-in profile IDs and contents are pinned by serialization fixtures.
2. Resolution rejects a disallowed or weaker profile and is monotonic under added requirements.
3. Strict and standard profiles never contain degraded controls.
4. Compatibility degradation is explicit, stable, and persisted.
5. Every runtime either returns matching evidence for all required controls or rejects before
   allocation.
6. VM runtimes satisfy the same semantic contract through equivalent mechanisms.
7. Partial enforcement failure performs complete cleanup and records failure.
8. Old execution manifests remain readable.
9. No adapter is described as hostile-multitenant qualified by Phase 0 alone.
