---
source_sha: f9aa9a79348227ca3c517fe7ea86b3bc6eb16829
status: proposed
sources:
  - crates/minibox-domain/src/lib.rs
  - crates/minibox-domain/src/runtime.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-domain/src/networking.rs
  - docs/designs/2026-09-21-global-fail-closed-isolation-profiles-design.md
generated: 2026-09-21
---

# Design: Isolation Profile Contract And Digests

## Goal

Define the portable, deterministic isolation vocabulary consumed by admission, runtimes,
protocols, manifests, and conformance tests.

## Ownership

- **Owner:** `minibox-domain`
- **New module:** `crates/minibox-domain/src/isolation.rs`
- **Consumers:** `minibox-core`, `minibox`, `macbox`, `miniboxd`, and `minibox-testsuite`
- **Dependencies:** no adapter crate and no new external dependency

## Profile IDs

```rust
pub enum IsolationProfileId {
    StandardV1,
    CompatibilityV1,
}

pub enum IsolationControl {
    Identity,
    Process,
    Mount,
    Network,
    LinuxCapabilities,
    NoNewPrivileges,
    SyscallFilter,
    Filesystem,
    Resources,
}
```

Both enums serialize with stable kebab-case names. `StandardV1` is the global default.

## Capability Contract

`LinuxCapability` stores the numeric kernel capability ID. Human-facing input accepts known
`CAP_*` names and `CAP_<decimal-number>`. Wire output may use names, but every digest projection
uses the numeric ID so a future name table update cannot change historical digests.

`DockerDefaultV1` is pinned to:

- `CAP_AUDIT_WRITE`
- `CAP_CHOWN`
- `CAP_DAC_OVERRIDE`
- `CAP_FOWNER`
- `CAP_FSETID`
- `CAP_KILL`
- `CAP_MKNOD`
- `CAP_NET_BIND_SERVICE`
- `CAP_NET_RAW`
- `CAP_SETFCAP`
- `CAP_SETGID`
- `CAP_SETPCAP`
- `CAP_SETUID`
- `CAP_SYS_CHROOT`

Changing this set requires a new baseline and profile version.

```rust
pub struct LinuxCapability(u16);

pub enum LinuxCapabilityBaseline {
    DockerDefaultV1,
}

pub struct LinuxCapabilityRequest {
    add: BTreeSet<LinuxCapability>,
    drop: BTreeSet<LinuxCapability>,
}

pub enum LinuxCapabilityGate {
    DenyAllAdditions,
    AllowListed(BTreeSet<LinuxCapability>),
    AllowAll,
}
```

Resolution applies baseline, then authorized additions, then drops. Drops win conflicts. Runtime
admission rejects values above the selected kernel's `cap_last_cap`.

## Digest Contract

```rust
pub struct IsolationDigest([u8; 32]);

pub enum IsolationDigestDomain {
    WorkloadV2,
    ProfileV1,
    RequestV1,
    PlanV1,
    BindingV1,
    EvidenceV1,
    CleanupV1,
    EnvironmentGenerationV1,
}

pub fn compute_isolation_digest<T: Serialize>(
    domain: IsolationDigestDomain,
    value: &T,
) -> Result<IsolationDigest, IsolationError>;
```

The algorithm is SHA-256 over:

1. ASCII prefix `minibox-isolation` followed by one NUL byte.
2. The kebab-case domain name length as unsigned big-endian `u32`.
3. The domain name bytes.
4. The schema version as unsigned big-endian `u32`.
5. The canonical payload length as unsigned big-endian `u64`.
6. Canonical JSON payload bytes.

Canonical JSON rules:

- UTF-8 only.
- Object keys sorted by UTF-8 byte order.
- No insignificant whitespace.
- Integers emitted in minimal decimal form; floating-point values are forbidden.
- Sets emitted as arrays sorted by each element's canonical byte representation.
- Maps emitted as sorted objects when keys are strings, otherwise as sorted key/value arrays.
- Optional absent fields are omitted; explicit `null` is forbidden in digest projections.
- Capability values use numeric IDs, not display names.
- Paths use validated UTF-8 internal-path form; non-UTF-8 host paths are rejected before policy
  resolution.

Text display is `sha256:<64 lowercase hexadecimal characters>`. Parsing accepts only that exact
form.

## Resolved Profile Shape

```rust
pub struct ResolvedIsolationProfile {
    schema_version: u32,
    profile: IsolationProfileId,
    identity: IdentityPolicy,
    process: ProcessPolicy,
    mount: MountPolicy,
    network: NetworkPolicy,
    capabilities: ResolvedLinuxCapabilities,
    no_new_privileges: NoNewPrivilegesPolicy,
    syscall_filter: SyscallFilterPolicy,
    filesystem: FilesystemPolicy,
    resources: ResourcePolicy,
    allowed_evidence_authorities:
        BTreeMap<IsolationControl, BTreeSet<EvidenceAuthority>>,
    policy_digest: IsolationDigest,
    operator_authorization: Option<OperatorAuthorizationId>,
}
```

Fields are private. Construction and deserialization use one validating constructor. Callers get
read-only accessors. A valid resolved profile contains each of the nine controls exactly once.

## Built-In Semantics

### `standard-v1`

- No degraded controls.
- Host-root identity is forbidden without a proven mapped or equivalent isolation boundary.
- Process, mount, and non-host network boundaries are required.
- Docker capability baseline plus authorized modifiers; inheritable and ambient sets are empty.
- `no_new_privileges` is required.
- The `standard-v1` syscall artifact and mount-immutability rule are required.
- Device nodes and setid bits are denied.
- Host mounts follow the operator allowlist and access mode.
- Requested resource limits are enforced; `pids_max` defaults to 1024.
- Evidence authority is profile-owned, not adapter-selected.

### `compatibility-v1`

- Requires caller selection and an `OperatorAuthorizationId`.
- The union of enforced and degraded entries contains all nine controls exactly once.
- Explicit capability add/drop requests still require exact enforcement.
- Every degradation includes a reason and operator authorization binding.
- Compatibility evidence never qualifies `standard-v1`.

### Legacy Privileged

Legacy privileged is a separate resolved mode, not a profile. It requires its own operator grant,
conflicts with profile controls, and remains unqualified.

## Serialization Safety

Request DTOs may derive `Deserialize` and remain untrusted. Resolved profiles, grants, digests,
plans, and evidence implement custom `Deserialize` through validating constructors. Unknown schema
or profile versions fail closed.

## Verification

- Golden vectors for every digest domain.
- Property tests for map/set ordering and deterministic output.
- Golden numeric capability projections independent of display names.
- Drop-wins and authorization monotonicity properties.
- Rejection tests for unknown versions, malformed paths, floats, nulls, and noncanonical digests.

## Out Of Scope

- Runtime lifecycle and rollback.
- Platform syscall implementation.
- Wire protocol and client migration.
- Cryptographic signatures and remote attestation.
