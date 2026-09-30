---
source_sha: f5481a9482fbb04690db6b7a52ee8eca9c5fe5e9
status: superseded
superseded_by: docs/designs/2026-09-18-environment-attested-isolation-design.md
sources:
  - crates/minibox-core/src/protocol.rs
  - crates/minibox-core/tests/protocol_evolution.rs
  - crates/minibox-core/tests/property_roundtrip.rs
  - crates/minibox/src/daemon/handler/mod.rs
  - crates/minibox/src/daemon/handler/run.rs
  - crates/minibox/src/daemon/handler/run/model.rs
  - crates/minibox/src/daemon/handler/run/preparation.rs
  - crates/minibox/src/daemon/server.rs
  - crates/miniboxd/src/config.rs
  - crates/miniboxd/src/main.rs
  - crates/miniboxd/src/adapter_registry.rs
  - docs/designs/2026-09-12-guaranteed-security-profiles-design.md
generated: 2026-09-18
---

# Design: Isolation Policy Admission

> Superseded by `docs/designs/2026-09-18-environment-attested-isolation-design.md`.

## Goal

Apply a daemon-controlled security ceiling to every container request, allow clients to select only
approved versioned profiles, and reject unsupported required controls before resource allocation.

## Approved Approach

Use server-ceiling plus request selection: the daemon defines the minimum and allowlist, requests may
select or strengthen an allowed built-in profile, and the selected runtime may enforce equivalent
controls or reject but never weaken the result.

## Dependency

This design depends on
`docs/designs/2026-09-18-typed-isolation-policy-contract-design.md`. It wires that contract into the
daemon and protocol without redefining profile, plan, or evidence semantics.

It remains a Phase-0 bridge to
`docs/designs/2026-09-12-guaranteed-security-profiles-design.md`. Signed admission later replaces
Phase-0 resolution and `IsolationRuntime` with its lossless parameterized compiler and secure runtime.
Hostile profiles never pass through Phase-0 compatibility degradation.

## Context Map

### Files To Modify

| File                                                | Purpose                     | Change                                                                    |
| --------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------- |
| `crates/minibox-core/src/protocol.rs`               | Client/daemon wire contract | Add a distinct isolated-run request while preserving `Run`                |
| `crates/minibox-core/tests/protocol_evolution.rs`   | Compatibility fixtures      | Pin additive request fields and response variants                         |
| `crates/minibox-core/tests/property_roundtrip.rs`   | Serialization properties    | Round-trip profile IDs, requests, and reports                             |
| `crates/minibox/src/daemon/isolation.rs`            | Admission and composition   | Resolve policy, validate plans/evidence, and bundle isolated dependencies |
| `crates/minibox/src/daemon/handler/isolated_run.rs` | Isolated run orchestration  | Perform isolation admission through `DynIsolationRuntime`                 |
| `crates/minibox/src/daemon/server.rs`               | Protocol dispatch           | Map preflight, list, and run requests to admission                        |
| `crates/miniboxd/src/config.rs`                     | Operator policy             | Load and validate minimum/allowed profile configuration                   |
| `crates/miniboxd/src/main.rs`                       | Composition root            | Construct and inject the daemon ceiling                                   |
| `crates/miniboxd/src/adapter_registry.rs`           | Adapter selection           | Reject adapters that cannot satisfy the configured minimum                |
| `xtask/protocol-drift.lock`                         | Contract hash ledger        | Accept intentional request/response protocol drift                        |

### Dependencies And Consumers

| File                                    | Relationship                                                                                  |
| --------------------------------------- | --------------------------------------------------------------------------------------------- |
| `crates/mbx/src/commands/run.rs`        | Existing `Run` construction remains source-compatible and is normalized to the daemon minimum |
| `crates/mcp/src/tools/containers.rs`    | Existing `Run` construction remains source-compatible and cannot bypass production admission  |
| `crates/minibox-crux-plugin/src/lib.rs` | Existing `Run` construction remains source-compatible and cannot bypass production admission  |
| `crates/macbox/src/lib.rs`              | Existing composition remains source-compatible but unqualified until a later adapter slice    |

Client-specific flags and schemas are intentionally deferred. This slice preserves old wire decoders
while making omission select the server minimum rather than bypass isolation policy.

### Existing Test Coverage

| Test                                                   | Current Coverage                | Required Extension                                                |
| ------------------------------------------------------ | ------------------------------- | ----------------------------------------------------------------- |
| `crates/minibox-core/tests/protocol_evolution.rs`      | Additive protocol compatibility | Omitted profile, explicit profile, preflight, and unknown values  |
| `crates/minibox-core/tests/property_roundtrip.rs`      | Request/response round trips    | Isolation request/report properties                               |
| `crates/minibox/tests/daemon_handler_failure_tests.rs` | Handler failure injection       | Denial before filesystem, network, cgroup, and runtime allocation |
| `crates/miniboxd/src/config.rs` inline tests           | Layered config and environment  | Strict defaults, invalid allowlists, and precedence               |
| `crates/miniboxd/tests/protocol_e2e_tests.rs`          | Daemon request behavior         | Default, explicit, degraded, and rejected profiles                |

### Reference Patterns

| File                                       | Pattern                                        |
| ------------------------------------------ | ---------------------------------------------- |
| `crates/minibox-core/src/protocol.rs`      | Tagged Serde variants and additive defaults    |
| `crates/minibox/src/daemon/handler/mod.rs` | Focused dependency sub-structures              |
| `crates/minibox/src/daemon/handler/run.rs` | Run ordering and cleanup ownership             |
| `crates/miniboxd/src/config.rs`            | System/user/environment configuration layering |

### Risk

- New protocol variants are additive; the existing `Run` shape and `HandlerDependencies` remain
  source-compatible.
- Production `miniboxd` maps legacy `Run` to the daemon minimum, which may reject runs that
  previously succeeded.
- Invalid isolation configuration becomes a startup error instead of a warning/default fallback.
- No client crate changes or new dependencies are included.

## Crate Ownership

- **Protocol owner: `minibox-core`** - transports profile selection and reports using domain types.
- **Admission owner: `minibox`** - resolves requests, calls the selected runtime, validates evidence,
  and owns transactional cleanup.
- **Configuration owner: `miniboxd`** - defines the operator ceiling and supplies it at composition.

Primary ownership remains in these three crates. The user-approved crate-limit waiver includes the
workspace `xtask/protocol-drift.lock` ledger required by the protocol gate.

## Precedence

The effective order is:

```text
daemon minimum and allowlist
    > adapter hard constraints
    > request profile and added requirements
    > compatibility-only optional degradation
```

The `>` operator means "cannot be weakened by." Adapter constraints may reject a policy or provide
a stronger mechanism; they cannot remove a required control. Requests may choose only allowlisted
profiles at least as strong as the daemon minimum. Optional degradation is legal only when the
resolved profile is `compatibility-v1`.

## Public API

### Protocol Additions

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationProfileName(pub String);

impl TryFrom<&IsolationProfileName> for IsolationProfileId {
    type Error = IsolationPolicyError;

    fn try_from(name: &IsolationProfileName) -> Result<Self, Self::Error>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolatedRunRequest {
    pub image: String,
    pub tag: Option<String>,
    pub command: Vec<String>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub ephemeral: bool,
    pub network: Option<NetworkMode>,
    pub env: Vec<String>,
    pub mounts: Vec<BindMount>,
    pub privileged: bool,
    pub name: Option<String>,
    pub tty: bool,
    pub entrypoint: Option<String>,
    pub user: Option<String>,
    pub auto_remove: bool,
    pub priority: Option<slashcrux::Priority>,
    pub urgency: Option<slashcrux::Urgency>,
    pub execution_context: Option<slashcrux::ExecutionContext>,
    pub platform: Option<String>,
    pub cgroup_parent: Option<String>,
    pub isolation: IsolationSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationSelection {
    pub profile: Option<IsolationProfileName>,
    pub additional_required_controls: BTreeSet<IsolationControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationPreflightInput {
    pub profile: Option<IsolationProfileName>,
    pub additional_required_controls: BTreeSet<IsolationControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IsolationAdmissionDecision {
    Allowed,
    Denied {
        code: IsolationDenialCode,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IsolationDenialCode {
    UnknownProfile,
    ProfileNotAllowed,
    WeakerThanMinimum,
    UnsupportedRequiredControl,
    InvalidPlan,
    InvalidEvidence,
    EnforcementFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationPreflightReport {
    pub decision: IsolationAdmissionDecision,
    pub requested_profile: IsolationProfileName,
    pub effective_policy: Option<ResolvedIsolationPolicy>,
    pub adapter: Option<String>,
    pub plan: Option<IsolationPlan>,
}
```

The following variants are added:

```rust
DaemonRequest::RunIsolated {
    request: IsolatedRunRequest,
}

DaemonRequest::PreflightIsolation {
    input: IsolationPreflightInput,
}

DaemonRequest::ListIsolationProfiles

DaemonResponse::IsolationPreflight {
    report: IsolationPreflightReport,
}

DaemonResponse::IsolationProfiles {
    profiles: Vec<IsolationProfile>,
    minimum_profile: IsolationProfileId,
    allowed_profiles: BTreeSet<IsolationProfileId>,
}

DaemonResponse::IsolationDenied {
    code: IsolationDenialCode,
    reason: String,
}
```

The existing `DaemonRequest::Run` fields do not change. In the production isolated server,
`Run` is normalized into `IsolatedRunRequest` with `profile: None` and no added requirements, so old
clients receive the daemon minimum rather than bypassing admission. `IsolationProfileName` remains
unvalidated on the wire so unknown values reach admission and produce `UnknownProfile` instead of a
generic Serde failure. `IsolationDenied` is terminal for both legacy and isolated runs and carries
the same stable denial code as preflight.

Preflight is advisory. A real run repeats resolution and planning because configuration, adapter,
and runtime capabilities may change between calls.

### Admission Service

These types live in `minibox::daemon::isolation`.

```rust
#[derive(Debug, Clone)]
pub struct IsolationAdmission {
    ceiling: IsolationPolicyCeiling,
}

impl IsolationAdmission {
    pub fn new(ceiling: IsolationPolicyCeiling) -> Result<Self, IsolationPolicyError>;

    pub fn ceiling(&self) -> &IsolationPolicyCeiling;

    pub fn resolve(
        &self,
        profile: Option<&IsolationProfileName>,
        additional_required_controls: BTreeSet<IsolationControl>,
    ) -> Result<ResolvedIsolationPolicy, IsolationPolicyError>;

    pub async fn preflight(
        &self,
        runtime: &dyn IsolationRuntime,
        profile: Option<&IsolationProfileName>,
        additional_required_controls: BTreeSet<IsolationControl>,
    ) -> IsolationPreflightReport;

    pub fn validate_evidence(
        &self,
        plan: &IsolationPlan,
        evidence: &IsolationEvidence,
    ) -> Result<(), IsolationPolicyError>;
}

#[derive(Clone)]
pub struct IsolationDeps {
    pub admission: Arc<IsolationAdmission>,
    pub runtime: DynIsolationRuntime,
}

#[derive(Clone)]
pub struct IsolatedHandlerDependencies {
    pub legacy: Arc<HandlerDependencies>,
    pub isolation: IsolationDeps,
}

pub async fn handle_isolated_run(
    request: IsolatedRunRequest,
    state: Arc<DaemonState>,
    deps: Arc<IsolatedHandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) -> anyhow::Result<()>;

pub async fn run_server_with_isolation<L, F>(
    listener: L,
    state: Arc<DaemonState>,
    dependencies: Arc<IsolatedHandlerDependencies>,
    require_root_auth: bool,
    shutdown: F,
) -> anyhow::Result<()>
where
    L: ServerListener,
    F: Future<Output = ()>;
```

`HandlerDependencies`, `RunParams`, and the existing `run_server` signature remain unchanged.
`miniboxd` uses only `run_server_with_isolation`. The legacy server is retained for source
compatibility and explicitly unqualified test/platform composition; it must not be selected by the
production daemon. No `PolicyOverride` field may alter isolation profiles or controls.

### Daemon Configuration

```rust
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct IsolationPolicyConfig {
    pub minimum_profile: IsolationProfileId,
    pub allowed_profiles: BTreeSet<IsolationProfileId>,
}

impl Default for IsolationPolicyConfig;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
struct IsolationPolicyConfigPatch {
    minimum_profile: Option<IsolationProfileName>,
    allowed_profiles: Option<Vec<IsolationProfileName>>,
}

impl TryFrom<IsolationPolicyConfig> for IsolationPolicyCeiling {
    type Error = ConfigError;

    fn try_from(config: IsolationPolicyConfig) -> Result<Self, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read daemon configuration from {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid daemon configuration at {path}: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("invalid isolation environment variable {name}: {value}")]
    InvalidIsolationEnvironment { name: String, value: String },
}
```

| Existing Type                      | Exact Change                                                                          |
| ---------------------------------- | ------------------------------------------------------------------------------------- |
| `PolicyConfig`                     | Add final `pub isolation: IsolationPolicyConfig` after all patches merge              |
| `DaemonConfig::load`               | Change signature to `pub fn load() -> Result<Self, ConfigError>`                      |
| `DaemonConfig::load_from_path`     | Change signature to `pub fn load_from_path(path: &Path) -> Result<Self, ConfigError>` |
| `DaemonConfig::with_env_overrides` | Change signature to `pub fn with_env_overrides(self) -> Result<Self, ConfigError>`    |

System and user files deserialize into an internal `DaemonConfigPatch`; its policy section contains
`IsolationPolicyConfigPatch`. Optional patch fields are merged first, environment values are applied
second, and `IsolationPolicyConfig::default()` is applied only once after layering. An omitted higher
layer therefore cannot overwrite an explicit lower-layer isolation setting with strict defaults.
`std::io::ErrorKind::NotFound` produces an empty patch; every other read error returns
`ConfigError::Read`. Existing malformed files return `ConfigError::Invalid` rather than silently
falling back.

The default is:

```text
minimum_profile = strict-v1
allowed_profiles = [strict-v1]
```

Operators may configure `MINIBOX_ISOLATION_MINIMUM` and a comma-separated
`MINIBOX_ISOLATION_ALLOWED`. Unknown IDs, an empty allowlist, a missing minimum in the allowlist, or
an allowlisted profile weaker than the minimum are startup errors.

### Adapter Registry Validation

```rust
pub async fn validate_adapter_isolation(
    runtime: &dyn IsolationRuntime,
    ceiling: &IsolationPolicyCeiling,
) -> Result<(), IsolationPolicyError>;
```

Startup validates that the selected adapter can plan the daemon minimum. Dynamic environment loss
still causes per-request rejection; startup validation is not treated as permanent capability
evidence.

## Data Flow

1. `miniboxd` loads layered configuration and validates `IsolationPolicyCeiling`.
2. The composition root validates the selected adapter against the minimum and injects immutable
   `IsolationDeps`.
3. The protocol decoder maps an omitted request profile to `None`, never to compatibility.
4. The run handler resolves the server minimum, selected profile, and added requirements.
5. The selected `IsolationRuntime` plans the resolved policy before rootfs, network, cgroup, VM, or
   process allocation.
6. A denial returns a stable `IsolationDenialCode`; no preparation function is called.
7. An allowed plan is persisted as a pending `ExecutionManifest` value in `DaemonState` before
   workload allocation, then passed unchanged to `IsolationRuntime::spawn_isolated`, which owns
   preparation and spawn.
8. The isolation runtime returns evidence; the admission service validates it before reporting
   success.
9. On success, the handler writes the enforced manifest to the returned container root and removes
   the pending state entry atomically. Invalid or failed enforcement triggers transactional cleanup
   and persists the pending value as a failed manifest record without requiring `container_dir` to
   exist.

## Hexagonal Boundaries

- `minibox-core` transports domain values but contains no policy resolution.
- `minibox::daemon::isolation` is the application service coordinating the Phase-0
  `IsolationRuntime` port.
- `miniboxd` is the configuration adapter and composition root.
- Clients request profiles; they do not define policy, compile controls, or weaken the ceiling.

## Failure Contract

- Decode/configuration errors fail daemon startup or reject the request with a stable code.
- Profile and required-control errors occur before adapter allocation.
- Adapters never silently ignore requested controls.
- Compatibility degradation appears in preflight, execution evidence, and the persisted manifest.
- Mid-setup failure invokes runtime, network, limiter, and filesystem cleanup in reverse allocation
  order.
- A successful spawn with missing or mismatched evidence returns `InvalidEvidence`; execution errors
  while applying a valid plan return `EnforcementFailed`.

The stable error mapping is:

| Domain/Application Error     | Denial Code                  |
| ---------------------------- | ---------------------------- |
| `UnknownProfile`             | `UnknownProfile`             |
| `ProfileNotAllowed`          | `ProfileNotAllowed`          |
| `WeakerThanMinimum`          | `WeakerThanMinimum`          |
| `UnsupportedRequiredControl` | `UnsupportedRequiredControl` |
| `InvalidPlan`                | `InvalidPlan`                |
| `InvalidEvidence`            | `InvalidEvidence`            |
| `EnforcementFailed`          | `EnforcementFailed`          |

Missing qualified isolation dependencies and `InvalidCeiling` are startup failures, not reachable
request denial codes.

## Verification Strategy

- Protocol fixtures prove old clients deserialize and select the daemon minimum.
- Property tests prove no request can resolve below the daemon minimum.
- Configuration tables cover system, user, and environment precedence plus every invalid allowlist.
- Failure-injection tests assert zero allocator calls after admission denial.
- Preflight and run tests prove run repeats planning rather than trusting stale preflight output.
- End-to-end tests cover strict success, disallowed profile, weaker profile, promoted requirements,
  compatibility degradation, and adapter capability loss.
- Tests assert internal pipeline policy cannot widen or select a weaker profile.

## Integration With Signed Profiles

The later signed admission design replaces Phase-0 resolution and `IsolationRuntime` authority after
authorization verification. Its parameterized `ControlRequirement` values are not reduced to
`additional_required_controls`; the signed `SecurityProfileCompiler` compiles them losslessly.
Hostile multi-tenant workloads therefore never use Phase-0 compatibility degradation.

Adapter mechanism labels, negative tests, rollback behavior, and conformance fixtures may migrate to
the signed runtime. Phase-0 manifest evidence migrates into `ExecutionSecurityRecord`; old
`ExecutionIsolationRecord` values remain readable as unauthenticated historical evidence.

## Out Of Scope

- CLI, MCP, and Crux profile-selection flags or schemas.
- Caller-authored profiles and platform-native policy payloads.
- Signing, trust configuration, replay prevention, and cryptographic attestation.
- Automatic fallback to a different adapter or profile.
- Rootless or hostile multi-tenant qualification claims.

## Compatibility And Risk

- Protocol variants are wire-additive, but adding variants to public enums is semver-significant for
  external exhaustive matches; strict default admission is also a deliberate semantic break.
- `DaemonConfig::load` and related functions become fallible public APIs.
- `HandlerDependencies`, `RunParams`, and legacy client constructors remain source-compatible.
- The protocol hash ledger changes under the approved crate-limit waiver.
- Existing internal `ContainerPolicy`, `ExecutionPolicy`, and MCP policy remain stricter outer gates;
  none may authorize a weaker isolation policy.
- No new dependency or feature flag is required.

## Acceptance Criteria

1. The daemon defaults to `strict-v1`; compatibility requires explicit operator allowlisting and
   request selection.
2. Requests cannot choose a disallowed or weaker profile and can only add requirements.
3. Admission denial occurs before any rootfs, network, cgroup, VM, or process allocation.
4. Strict and standard runs never report degraded controls.
5. Compatibility degradation is visible in preflight, evidence, and manifests.
6. Adapter hard constraints may strengthen or reject but never weaken the resolved policy.
7. Old protocol clients remain decodable and receive the daemon minimum.
8. Invalid isolation configuration fails startup.
9. Signed hostile profiles later replace Phase-0 resolution with the lossless signed compiler while
   retaining reusable mechanism labels, rollback tests, and historical evidence readers.
