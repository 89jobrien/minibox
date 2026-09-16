---
source_sha: 09902733a48219449a6569c01a3f8c5a67ba0c09
sources:
  - Cargo.toml
  - crates/minibox-domain/Cargo.toml
  - crates/minibox-domain/src/runtime.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-domain/src/execution_policy.rs
  - crates/minibox-domain/src/events.rs
  - crates/minibox-domain/src/workflow.rs
  - crates/minibox-core/src/protocol.rs
  - crates/minibox/src/container/process.rs
  - crates/minibox/src/container/namespace.rs
  - crates/minibox/src/container/mount_seccomp.rs
  - crates/minibox/src/adapters/mod.rs
  - crates/minibox/src/adapters/runtime.rs
  - crates/minibox/src/adapters/gke.rs
  - crates/minibox/src/adapters/colima.rs
  - crates/minibox/src/adapters/smolvm.rs
  - crates/minibox/src/adapters/hcs.rs
  - crates/minibox/src/adapters/wsl2.rs
  - crates/minibox/src/daemon/handler/mod.rs
  - crates/minibox/src/daemon/handler/run.rs
  - crates/minibox/src/daemon/handler/manifest.rs
  - crates/minibox/src/daemon/handler/image.rs
  - crates/minibox/src/daemon/handler/exec.rs
  - crates/minibox/src/daemon/handler/update.rs
  - crates/minibox/src/daemon/handler/snapshot.rs
  - crates/minibox/src/adapters/builder.rs
  - crates/miniboxd/src/config.rs
  - crates/miniboxd/src/main.rs
  - crates/miniboxd/src/adapter_registry.rs
  - crates/macbox/src/krun/runtime.rs
  - crates/macbox/src/vz/adapter.rs
  - crates/smolbox/src/lib.rs
  - crates/smolbox/src/smolvm/mod.rs
  - crates/smolbox/src/krun/mod.rs
  - crates/winbox/src/lib.rs
  - crates/mbx/src/commands/run.rs
  - crates/mbx/src/commands/sandbox.rs
  - crates/mbx/src/commands/pipeline.rs
  - crates/mcp/src/policy.rs
  - crates/mcp/src/tools/containers.rs
  - crates/mcp/src/types.rs
  - crates/minibox-crux-plugin/src/lib.rs
  - crates/minibox-testsuite/src/adapters/runtime.rs
generated: 2026-09-12
---

# Design: Guaranteed Security Profiles

## Goal

Provide signed, fail-closed, adapter-independent security profiles that let Minibox run hostile
multi-tenant workloads only when the selected adapter can prove every required guarantee.

## Approved Approach

Use the **Contract-First Profile Compiler** approach: define portable security guarantees in the
domain, verify signed workload authorizations, compile them into adapter-specific enforcement
plans, and reject any run that cannot be enforced without downgrade.

## Explicit Constraints

- Security is enabled by default when this program ships; unsigned execution is not retained as a
  compatibility path.
- Policy bundles are immutable by digest and may be embedded in, or referenced by, a signed
  workload authorization manifest.
- Local Ed25519 keys and Sigstore identities are both supported trust modes in the first release.
- Every runtime adapter participates in the contract by enforcing a profile or rejecting it before
  workload resources are allocated.
- Privileged execution is a separately signed profile class, not a bypass around admission.
- CLI, MCP, and Crux use the same daemon-owned admission and enforcement path.
- The target threat model is hostile multi-tenancy: arbitrary code, malicious images, and mutually
  untrusted tenants sharing infrastructure.

## Contents

- [Threat Model And Assumptions](#threat-model-and-assumptions)
- [Context Map](#context-map)
- [Delivery Slices](#delivery-slices)
- [Crate Ownership](#crate-ownership)
- [Public API](#public-api)
- [Data Flow](#data-flow)
- [Hexagonal Boundaries](#hexagonal-boundaries)
- [Adapter Enforcement Model](#adapter-enforcement-model)
- [Failure, Cleanup, And Audit Semantics](#failure-cleanup-and-audit-semantics)
- [Verification Strategy](#verification-strategy)
- [External Dependencies And Features](#external-dependencies-and-features)
- [Integration And Compatibility](#integration-and-compatibility)
- [Out Of Scope](#out-of-scope)
- [Risk](#risk)
- [Acceptance Criteria](#acceptance-criteria)

## Threat Model And Assumptions

The design protects the host and peer tenants from a workload that controls its image, command,
environment, process tree, and network traffic. It assumes the host kernel, hypervisor, daemon,
configured trust roots, and policy-bundle store are trusted and patched. A guarantee describes a
specific enforceable property; it is not a claim that a shared kernel or hypervisor has no unknown
vulnerabilities.

Daemon clients are trusted schedulers, not tenant-controlled processes. Tenant workloads do not
receive daemon socket access, inherited daemon file descriptors, or host credentials. Per-tenant
control-plane authentication is a separate concern; a deployment that gives tenants direct daemon
access is not qualified by this design.

Image build, exec, restart, snapshot restore, pipeline, and workflow paths are considered separate
execution entry points rather than harmless lifecycle helpers. A hostile-multitenant deployment
disables an entry point until it either uses this admission path or has a narrower design proving
that it inherits an already-enforced plan without widening it.

The hostile-multi-tenant qualification excludes physical attacks, compromised host administrators,
stolen signing keys, microarchitectural side channels, confidential-computing attestation, and
denial of service outside configured resource controllers.

### Terminology

| Term                   | Meaning                                                                          |
| ---------------------- | -------------------------------------------------------------------------------- |
| Profile class          | `Standard` or explicitly weaker `Privileged` authorization category              |
| Policy bundle          | Immutable, digest-addressed set of required security controls                    |
| Control                | Portable outcome such as identity separation, syscall filtering, or PID limits   |
| Workload authorization | Signed statement binding principal, execution ID, workload, and policy digests   |
| Enforcement plan       | Adapter-specific compilation of every policy control for one normalized workload |
| Enforcement evidence   | Adapter or guest proof that the bound plan was applied before execution          |

## Context Map

### Files To Modify: Contract And Trust

| File                                              | Purpose                               | Changes Needed                                                                             |
| ------------------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------ |
| `Cargo.toml`                                      | Workspace membership and dependencies | Add `minibox-security` and shared dependency entries                                       |
| `crates/minibox-domain/src/security.rs`           | New portable security contract        | Add authorization, policy, plan, evidence, audit, replay, and port types                   |
| `crates/minibox-domain/src/lib.rs`                | Domain exports                        | Export security types and dynamic port aliases                                             |
| `crates/minibox-domain/src/runtime.rs`            | Existing raw runtime port             | Keep raw spawn primitive; document that daemon run admission uses `SecureContainerRuntime` |
| `crates/minibox-domain/src/execution_manifest.rs` | Persisted execution evidence          | Add an optional security record and bind it into the execution digest                      |
| `crates/minibox-domain/src/execution_policy.rs`   | Existing post-hoc policy              | Deprecate as authorization; retain only legacy manifest inspection during migration        |
| `crates/minibox-security/Cargo.toml`              | New trust adapter crate               | Isolate canonical JSON, Ed25519, Sigstore, and durable storage dependencies                |
| `crates/minibox-security/src/lib.rs`              | Security adapter exports              | Export canonicalization and concrete port constructors                                     |
| `crates/minibox-security/src/canonical.rs`        | Canonical payloads and digests        | Implement RFC 8785 canonicalization and SHA-256 content digests                            |
| `crates/minibox-security/src/signing.rs`          | Client-side signing adapters          | Implement local Ed25519 and Sigstore signing                                               |
| `crates/minibox-security/src/verification.rs`     | Daemon-side trust adapters            | Verify local signatures, Sigstore bundles, identities, validity, and audience              |
| `crates/minibox-security/src/policy_store.rs`     | Immutable policy resolution           | Resolve embedded or content-addressed bundles and verify their digest                      |
| `crates/minibox-security/src/replay_store.rs`     | Authorization replay state            | Atomically claim nonces and persist resumable execution ownership                          |
| `crates/minibox-security/src/audit.rs`            | Durable security audit                | Append owner-only, redacted JSONL records and fail closed on write errors                  |

### Files To Modify: Daemon Admission And Protocol

| File                                              | Purpose                          | Changes Needed                                                                                           |
| ------------------------------------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `crates/minibox-core/src/protocol.rs`             | Canonical client/daemon protocol | Carry signed authorization, add preflight/profile requests, and add structured responses                 |
| `crates/minibox-core/tests/protocol_evolution.rs` | Wire compatibility               | Snapshot additive fields and variants                                                                    |
| `crates/minibox-core/tests/property_roundtrip.rs` | Serialization properties         | Round-trip authorization and security responses                                                          |
| `crates/minibox/src/daemon/security.rs`           | New admission transaction        | Normalize requests, verify authorization, claim replay state, resolve policy, compile, audit, and launch |
| `crates/minibox/src/daemon/handler/mod.rs`        | Injected handler ports           | Add `SecurityDeps`; retire policy overrides from run paths                                               |
| `crates/minibox/src/daemon/handler/run.rs`        | Current run orchestration        | Route all process creation through admission before rootfs/network/cgroup/VM/process allocation          |
| `crates/minibox/src/daemon/handler/pipeline.rs`   | Internal pipeline execution      | Remove unsigned `PolicyOverride` widening and propagate signed authorization                             |
| `crates/minibox-domain/src/workflow.rs`           | Sequential workflow definitions  | Require authorization inside each `container-run` step configuration                                     |
| `crates/minibox/src/daemon/handler/update.rs`     | Image update and restart         | Reject restart for secured containers until replacement authorization is supported                       |
| `crates/minibox/src/daemon/handler/snapshot.rs`   | VM snapshot restore              | Reject restore for secured containers until explicit authorized resumption is implemented                |
| `crates/minibox/src/daemon/handler/manifest.rs`   | Post-run inspection              | Return security evidence; stop treating caller policy JSON as authorization                              |
| `crates/minibox/src/daemon/handler/image.rs`      | Image operations                 | Return `OperationDisabled` for Dockerfile build; retain non-executing pull/push/commit paths             |
| `crates/minibox/src/daemon/handler/exec.rs`       | New process in a container       | Return `OperationDisabled` until exec can inherit and prove the stored plan                              |
| `crates/minibox/src/adapters/builder.rs`          | Raw runtime-backed image build   | Remove from daemon dependency injection; retain only for non-daemon tests until redesigned               |
| `crates/minibox/src/daemon/server.rs`             | Protocol dispatch                | Route preflight/profile requests and pass authorization into run admission                               |
| `crates/miniboxd/Cargo.toml`                      | Composition dependencies         | Depend on `minibox-security` with both trust modes enabled                                               |
| `crates/miniboxd/src/config.rs`                   | Trust and storage config         | Add fail-closed security configuration; reject invalid configured trust files                            |
| `crates/miniboxd/src/main.rs`                     | Composition root                 | Build trust, resolver, replay, audit, and secure-runtime adapters                                        |
| `crates/miniboxd/src/adapter_registry.rs`         | Adapter selection                | Reject selectable adapters that do not provide a secure runtime                                          |

### Files To Modify: Adapter Enforcement

| File                                            | Purpose                     | Changes Needed                                                                         |
| ----------------------------------------------- | --------------------------- | -------------------------------------------------------------------------------------- |
| `crates/minibox/src/adapters/runtime.rs`        | Native Linux runtime        | Add the native profile compiler/enforcer wrapper                                       |
| `crates/minibox/src/container/namespace.rs`     | Namespace creation          | Add user namespace flags and parent/child mapping synchronization                      |
| `crates/minibox/src/container/process.rs`       | Native child initialization | Apply mapped identity, capability ceiling, seccomp plan, and enforcement handshake     |
| `crates/minibox/src/container/mount_seccomp.rs` | Existing mount-only filter  | Compose the mount ratchet into the general seccomp plan                                |
| `crates/minibox/src/adapters/gke.rs`            | GKE/proot runtime           | Require verified outer-sandbox evidence or reject hostile profiles                     |
| `crates/minibox/src/adapters/colima.rs`         | Shared Lima runtime         | Require a dedicated qualifying VM or reject hostile profiles                           |
| `crates/minibox/src/adapters/smolvm.rs`         | SmolVM runtime              | Add guest capability handshake, plan delivery, and evidence return                     |
| `crates/macbox/src/krun/runtime.rs`             | krun runtime                | Add guest capability handshake, plan delivery, and evidence return                     |
| `crates/macbox/src/vz/adapter.rs`               | VZ runtime                  | Replace nested unsigned `DaemonRequest::Run` with guest plan enforcement               |
| `crates/minibox/src/adapters/hcs.rs`            | HCS stub                    | Implement Hyper-V-isolated profile mapping before qualification                        |
| `crates/minibox/src/adapters/wsl2.rs`           | WSL2 adapter                | Implement explicit rejection for hostile profiles unless a stronger boundary is proven |
| `crates/minibox/src/adapters/docker_desktop.rs` | Library-only VM adapter     | Implement the contract as unqualified/rejecting until dedicated isolation exists       |
| `crates/minibox/src/adapters/vf.rs`             | Library-only VZ predecessor | Implement the contract as unqualified/rejecting or retire the stub                     |
| `crates/winbox/src/lib.rs`                      | Windows composition         | Wire HCS secure runtime and authenticated Named Pipe admission; keep WSL2 weaker       |
| `crates/smolbox/src/smolvm/mod.rs`              | SmolVM facade re-export     | Re-export the secure SmolVM runtime from `minibox`                                     |
| `crates/smolbox/src/krun/mod.rs`                | krun facade re-export       | Re-export the secure krun runtime from `macbox`                                        |
| `crates/smolbox/tests/conformance_reexports.rs` | Facade conformance          | Prove secure runtime exports remain available through `smolbox`                        |

### Files To Modify: Control Surfaces

| File                                         | Purpose                    | Changes Needed                                                                  |
| -------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------- |
| `crates/mbx/src/main.rs`                     | CLI command model          | Require authorization for run and add security sign/inspect/preflight commands  |
| `crates/mbx/src/commands/run.rs`             | CLI run mapping            | Load authorization and send it unchanged to the daemon                          |
| `crates/mbx/src/commands/sandbox.rs`         | Sandboxed script execution | Require signed authorization rather than constructing an unsigned `Run` request |
| `crates/mbx/src/commands/pipeline.rs`        | Pipeline client            | Carry the pipeline container authorization                                      |
| `crates/mbx/src/commands/security.rs`        | New security CLI           | Create signed authorizations and inspect/preflight immutable profiles           |
| `crates/mcp/src/types.rs`                    | MCP schemas                | Add required signed authorization to run input and security report output       |
| `crates/mcp/src/tools/containers.rs`         | MCP run mapping            | Preserve defense-in-depth checks while forwarding authorization unchanged       |
| `crates/mcp/src/policy.rs`                   | MCP boundary policy        | Prevent agent permissions from weakening signed daemon policy                   |
| `crates/mcp/src/server.rs`                   | MCP tool router            | Expose profile inspection and preflight, but no signing-key access              |
| `crates/minibox-crux-plugin/src/lib.rs`      | Crux handler mapping       | Require authorization for run and expose preflight/profile handlers             |
| `crates/minibox-crux-plugin/src/protocol.rs` | Crux plugin wire envelope  | Continue carrying JSON values; declarations document the signed schema          |

### Dependencies And Consumers

| File                                              | Relationship                                                                                          |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `crates/minibox-domain/src/runtime.rs`            | `ContainerRuntime` remains the low-level process primitive wrapped by secure runtimes                 |
| `crates/minibox/src/daemon/handler/mod.rs`        | Every adapter-suite builder constructs `HandlerDependencies`; all sites must gain `SecurityDeps`      |
| `crates/miniboxd/src/main.rs`                     | Selectable adapter suites are composed here and must provide matching compiler/enforcer objects       |
| `crates/macbox/src/lib.rs`                        | Independently constructs handler dependencies for macOS paths                                         |
| `crates/minibox-core/src/protocol.rs`             | Every direct `DaemonRequest::Run` constructor must add authorization or use an internal non-wire path |
| `crates/minibox-domain/src/execution_manifest.rs` | Existing digest and environment hashing are reused, then upgraded to canonical security digests       |
| `crates/minibox/src/daemon/handler/pipeline.rs`   | Current trusted override is an authorization bypass and must be removed                               |
| `crates/mcp/src/policy.rs`                        | Agent policy stays as a stricter outer gate, never an authority to widen daemon policy                |
| `crates/minibox/src/daemon/handler/update.rs`     | Current restart path calls run internals and must not bypass fresh admission                          |
| `crates/minibox/src/daemon/handler/snapshot.rs`   | Restore can resume execution and therefore requires authorization revalidation                        |
| `crates/minibox/src/daemon/handler/image.rs`      | Current `Build` path executes untrusted Dockerfile commands through an image builder                  |
| `crates/minibox/src/daemon/handler/exec.rs`       | Current `Exec` path creates a process that does not prove inherited plan enforcement                  |
| `crates/minibox/src/adapters/builder.rs`          | Wraps the raw runtime and must not be reachable from daemon handlers                                  |
| `crates/mbx/src/commands/sandbox.rs`              | Constructs `DaemonRequest::Run` independently of the main run command                                 |
| `crates/mbx/src/commands/pipeline.rs`             | Constructs `DaemonRequest::RunPipeline` independently of the main run command                         |

### Existing Test Coverage And Required Extensions

| Test Location                                            | Current Coverage                                       | Required Extension                                                        |
| -------------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------- |
| `crates/minibox-domain/src/execution_manifest.rs`        | Digest stability, environment hashing, serialization   | Canonical workload and security-evidence binding                          |
| `crates/minibox-domain/src/execution_policy.rs`          | Post-hoc image/network/privilege/mount checks          | Migration tests proving it cannot authorize execution                     |
| `crates/minibox-core/tests/protocol_evolution.rs`        | Request/response compatibility                         | Signed authorization, profile reports, and denial codes                   |
| `crates/minibox-core/tests/property_roundtrip.rs`        | Protocol property round-trips                          | Security contracts and unknown-field rejection                            |
| `crates/minibox/src/container/mount_seccomp.rs`          | Mount-ratchet BPF model and Linux kernel test          | General default-deny seccomp composition and plan binding                 |
| `crates/minibox/tests/security_regression.rs`            | Extraction, FD, environment, request, and image limits | Capability, userns, seccomp, rootless, and downgrade regressions          |
| `crates/minibox/tests/native_adapter_isolation_tests.rs` | Native namespaces, cgroups, and overlay behavior       | Cross-tenant identity, process, network, filesystem, and resource attacks |
| `crates/minibox/tests/gke_adapter_isolation_tests.rs`    | Existing GKE isolation behavior                        | Outer-sandbox attestation and fail-closed rejection                       |
| `crates/minibox/tests/adapter_colima_tests.rs`           | Colima mapping and failures                            | Shared-VM rejection and dedicated-VM evidence                             |
| `crates/macbox/tests/krun_adapter_conformance.rs`        | krun runtime conformance                               | Guest profile handshake and evidence digest                               |
| `crates/macbox/tests/vz_adapter_smoke.rs`                | VZ smoke behavior                                      | Signed plan propagation and guest enforcement                             |
| `crates/minibox-testsuite/src/adapters/runtime.rs`       | Raw runtime conformance using mocks                    | New real-adapter secure-runtime conformance suite                         |
| `crates/mcp/tests/integration.rs`                        | MCP policy and protocol mapping                        | Authorization forwarding and no-signing-key exposure                      |
| `crates/miniboxd/tests/protocol_e2e_tests.rs`            | Daemon protocol end to end                             | Denial before allocation and durable audit outcomes                       |

### Reference Patterns

| File                                               | Pattern To Follow                                                       |
| -------------------------------------------------- | ----------------------------------------------------------------------- |
| `crates/minibox-domain/src/runtime.rs`             | Domain-owned object-safe ports and dynamic `Arc` aliases                |
| `crates/minibox/src/daemon/handler/mod.rs`         | Focused dependency sub-structures injected into handlers                |
| `crates/minibox-core/src/protocol.rs`              | Tagged Serde wire types with additive `#[serde(default)]` fields        |
| `crates/minibox-domain/src/execution_manifest.rs`  | Redacted environment representation and deterministic workload identity |
| `crates/minibox/src/container/mount_seccomp.rs`    | Pure filter construction plus model and real-kernel tests               |
| `crates/mcp/src/policy.rs`                         | Unforgeable authorization wrapper at a client-side safety boundary      |
| `crates/minibox-testsuite/src/adapters/runtime.rs` | Shared conformance registration and failure-path assertions             |

### Context Risks

- [x] Public API change: a new required secure-runtime port affects every selectable adapter.
- [x] Semantic protocol break: `Run.authorization` is additive on the wire but missing values are
      rejected once secure admission ships.
- [x] Persisted schema change: execution manifests, replay claims, policy bundles, and audit records
      are versioned and require compatibility fixtures.
- [x] Cross-crate boundary: the program spans domain, trust adapters, daemon, platform adapters, and
      clients, so delivery is divided into ordered slices below.
- [x] Existing bypass: `PolicyOverride`, optional `execution_policy`, and direct raw runtime access
      cannot remain on daemon run paths.
- [x] Secondary launch paths: update restart, snapshot restore, sandbox, pipeline, workflow steps,
      image build, and exec must route through admission, inherit a bound plan, or be disabled.
- [x] Adapter mismatch: compiler and enforcer must be supplied by the same secure-runtime object.
- [x] New external dependencies: cryptographic and canonicalization dependencies are isolated in
      `minibox-security`.

## Delivery Slices

The program design covers all adapters, but implementation is split so each slice has one coherent
responsibility and no slice claims production security before its vertical path is complete.

1. **Contract slice** (`minibox-domain`, `minibox-security`): domain contracts, canonicalization,
   test vectors, typed failures, and architecture-ring checks.
2. **Trust slice** (`minibox-security`, `mbx`, `miniboxd`): local Ed25519 and Sigstore signing and
   verification, immutable bundle resolution, replay storage, durable audit, and fail-closed startup.
3. **Admission slice** (`minibox-core`, `minibox`, `miniboxd`): protocol transport, request
   normalization, workload binding, audit-before-launch, and mandatory secure-runtime injection.
4. **Client slice** (`mbx`, `minibox-mcp`, `minibox-crux-plugin`): authorization forwarding,
   inspection, and preflight without duplicated enforcement logic or agent access to signing keys.
5. **Native slice** (`minibox-domain`, `minibox`, `minibox-testsuite`): user namespaces, capability
   ceiling, general seccomp, rootless resources, enforcement handshake, and hostile-workload tests.
6. **VM slice** (`minibox`, `macbox`, `smolbox`): SmolVM, krun, VZ, and Colima guest protocol,
   dedicated-boundary rules, evidence, and negative qualification tests.
7. **Managed/Windows slice** (`minibox`, `miniboxd`, `winbox`): GKE outer attestation, HCS/Hyper-V
   mapping, WSL2 rejection rules, and authenticated Windows daemon composition.
8. **Retirement and qualification slice**: remove unsigned widening paths, run the complete
   adversarial matrix, and publish only adapter/profile combinations with verified evidence.

Once mandatory admission lands, an adapter without a `SecureContainerRuntime` is unavailable for
run operations. The implementation may be integrated in slices, but the first public release of
this program must include both trust modes and must not expose an unsigned fallback.

## Crate Ownership

- **Contract owner: `minibox-domain`** - owns portable values and ports with no crypto, filesystem,
  Sigstore, kernel, VM, or HCS implementation dependency.
- **Trust adapter owner: `minibox-security`** - new crate whose single responsibility is canonical
  security payloads, signing/verification adapters, immutable policy resolution, replay storage,
  and durable security audit.
- **Application owner: `minibox`** - owns the admission transaction and native/platform secure
  runtime wrappers because current daemon orchestration and runtime adapters already live there.
- **Composition owner: `miniboxd`** - loads fail-closed operator configuration and supplies matching
  trust, storage, and secure-runtime adapters.
- **Platform owners: `minibox`, `macbox`, and `winbox`** - own guest or host-specific enforcement
  implementations without redefining the domain contract. `smolbox` remains the compatibility
  facade that re-exports SmolVM from `minibox` and krun from `macbox`.
- **Client owners: `mbx`, `minibox-mcp`, and `minibox-crux-plugin`** - transport signed
  authorization and display decisions; they do not enforce profiles.

### Dependency Direction

```text
A -> B means A depends on B.

minibox-core     -> minibox-domain
minibox-security -> minibox-domain
minibox          -> minibox-core, minibox-security
macbox           -> minibox-domain, minibox-security
smolbox          -> minibox, macbox
winbox           -> minibox-domain, minibox-security
miniboxd         -> minibox, minibox-security, platform crates
```

`minibox-security` depends on `minibox-domain`, never on `minibox-core` or `minibox`. Platform
adapters depend on the domain ports and may use `minibox-security` canonical digest helpers. This
keeps crypto and Sigstore dependencies out of the domain and avoids a dependency cycle.

## Public API

All structures below use named fields, reject unknown fields at signed boundaries, and use the
workspace's documented Rust naming and error conventions. Signatures are design contracts only.

### Domain Authorization And Digest Types

```rust
// minibox_domain::security

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DigestAlgorithm {
    Sha256,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CanonicalizationAlgorithm {
    Rfc8785,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HashedEnvironmentVariable {
    pub name: String,
    pub value_digest: ContentDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedMount {
    pub host_path: String,
    pub container_path: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedHook {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedWorkload {
    pub image_ref: String,
    pub image_digest: ContentDigest,
    pub command: Vec<String>,
    pub environment: Vec<HashedEnvironmentVariable>,
    pub mounts: Vec<NormalizedMount>,
    pub host_hooks: Vec<NormalizedHook>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub pids_max: Option<u64>,
    pub io_max_bytes_per_sec: Option<u64>,
    pub network: NetworkMode,
    pub name: Option<String>,
    pub ephemeral: bool,
    pub tty: bool,
    pub entrypoint: Option<String>,
    pub user: Option<String>,
    pub auto_remove: bool,
    pub platform: Option<String>,
    pub cgroup_parent: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProfileClass {
    Standard,
    Privileged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyBundleRef {
    Embedded {
        bundle: SecurityPolicyBundle,
    },
    Stored {
        name: String,
        digest: ContentDigest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizationClaims {
    pub execution_id: String,
    pub issuer: String,
    pub subject: String,
    pub tenant: String,
    pub audience: String,
    pub issued_at_unix_seconds: i64,
    pub not_before_unix_seconds: Option<i64>,
    pub expires_at_unix_seconds: i64,
    pub nonce: String,
    pub workload_digest: ContentDigest,
    pub policy_digest: ContentDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkloadAuthorizationManifest {
    pub schema_version: u32,
    pub canonicalization: CanonicalizationAlgorithm,
    pub workload: NormalizedWorkload,
    pub policy: PolicyBundleRef,
    pub claims: AuthorizationClaims,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthorizationSignature {
    Ed25519 {
        key_id: String,
        signature_base64: String,
    },
    Sigstore {
        bundle: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignatureScheme {
    Ed25519,
    Sigstore,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedWorkloadAuthorization {
    pub manifest: WorkloadAuthorizationManifest,
    pub signatures: Vec<AuthorizationSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerifiedSigner {
    LocalKey {
        key_id: String,
    },
    SigstoreIdentity {
        issuer: String,
        subject: String,
        transparency_log_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizationPrincipal {
    pub issuer: String,
    pub subject: String,
    pub tenant: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedAuthorization {
    pub execution_id: String,
    pub authorization_digest: ContentDigest,
    pub workload_digest: ContentDigest,
    pub policy_digest: ContentDigest,
    pub trust_policy_digest: ContentDigest,
    pub principal: AuthorizationPrincipal,
    pub signers: Vec<VerifiedSigner>,
    pub expires_at_unix_seconds: i64,
}
```

`NormalizedWorkload` is derived from effective daemon inputs, not directly trusted from a caller.
Environment values are hashed before comparison. Scheduling metadata that does not affect workload
execution remains outside this signed structure.

### Domain Policy Types

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum TenantIsolation {
    TrustedSingleTenant,
    HostileMultiTenant,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RootIdentity {
    HostRootAllowed,
    HostRootForbidden,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DaemonPrivilege {
    RootAllowed,
    RootlessRequired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum KernelBoundary {
    SharedKernel,
    DedicatedKernel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityControl {
    pub root_identity: RootIdentity,
    pub daemon_privilege: DaemonPrivilege,
    pub kernel_boundary: KernelBoundary,
    pub container_uid: u32,
    pub container_gid: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum KernelOperationClass {
    BasicIo,
    Filesystem,
    Process,
    Network,
    Ipc,
    ClockAdmin,
    NamespaceAdmin,
    MountAdmin,
    KernelAdmin,
    ModuleAdmin,
    Trace,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyscallDefaultAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyscallControl {
    pub default_action: SyscallDefaultAction,
    pub allowed_operations: Vec<KernelOperationClass>,
    pub denied_operations: Vec<KernelOperationClass>,
    pub mount_immutability: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrivilegeClass {
    FileOwnership,
    ProcessIdentity,
    NetworkAdmin,
    SystemAdmin,
    Audit,
    RawIo,
    KernelModule,
    SystemBoot,
    MacAdmin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityControl {
    pub allowed: Vec<PrivilegeClass>,
    pub no_new_privileges: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HostMountPolicy {
    Deny,
    ReadOnly,
    AllowListed {
        prefixes: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilesystemControl {
    pub read_only_rootfs: bool,
    pub deny_device_nodes: bool,
    pub deny_setid_bits: bool,
    pub host_mounts: HostMountPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkIsolation {
    None,
    Private,
    Dedicated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EgressPolicy {
    Deny,
    Allow,
    AllowListed {
        cidrs: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkControl {
    pub isolation: NetworkIsolation,
    pub egress: EgressPolicy,
    pub host_network_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceControl {
    pub memory_max_bytes: Option<u64>,
    pub pids_max: Option<u64>,
    pub cpu_weight_max: Option<u64>,
    pub io_max_bytes_per_sec: Option<u64>,
    pub require_enforcement: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostIntegrationControl {
    pub allow_lifecycle_hooks: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ControlKind {
    Identity,
    Syscalls,
    Capabilities,
    Filesystem,
    Network,
    Resources,
    HostIntegration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ControlRequirement {
    Identity(IdentityControl),
    Syscalls(SyscallControl),
    Capabilities(CapabilityControl),
    Filesystem(FilesystemControl),
    Network(NetworkControl),
    Resources(ResourceControl),
    HostIntegration(HostIntegrationControl),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SecurityPolicyBundle {
    pub schema_version: u32,
    pub name: String,
    pub class: ProfileClass,
    pub target_isolation: TenantIsolation,
    pub controls: Vec<ControlRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyBundleSummary {
    pub name: String,
    pub digest: ContentDigest,
    pub class: ProfileClass,
    pub target_isolation: TenantIsolation,
}
```

Policy validation rejects duplicate control kinds, contradictory identity requirements, empty
signatures, invalid time windows, malformed digests, unknown signed fields, and privileged request
fields paired with a standard profile.

### Mandatory Profile Invariants

Signing proves who authorized a bundle; it does not make a weak bundle secure. Validation applies
these non-overridable minimums before a signer scope or adapter is considered:

| Bundle Classification | Mandatory Invariants                                                                                                                                                                                                                                                                                                           |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Standard`            | Forbid host-root workload identity; require `no_new_privileges`; deny kernel-module, system-boot, MAC-admin, raw-I/O, and unrestricted system-admin privilege classes; forbid host networking unless the bundle targets `TrustedSingleTenant`                                                                                  |
| `Privileged`          | Require `TrustedSingleTenant`; require an explicitly trusted signer scope that includes `Privileged`; continue denying kernel-module, system-boot, and MAC-admin classes                                                                                                                                                       |
| `HostileMultiTenant`  | Require `Standard`; require immutable image digest; require host-root separation; require default-deny syscalls; require private or dedicated networking; forbid host lifecycle hooks; forbid host mounts or restrict them to read-only allowlists; deny device nodes and setid bits; require enforced memory and PID ceilings |

An embedded custom bundle may tighten these invariants but cannot weaken them. A trusted signer may
authorize custom values only within its `SignerPolicyScope`; the validator intersects signer scope,
profile-class baseline, and requested controls rather than choosing one of them.

```rust
pub fn validate_security_policy_bundle(
    bundle: &SecurityPolicyBundle,
) -> Result<(), SecurityError>;
```

### Domain Compilation And Evidence Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterIdentity {
    pub name: String,
    pub version: String,
    pub environment_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvidenceKind {
    LocalKernel,
    GuestAgent,
    OuterSandbox,
    Hypervisor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityCapabilities {
    pub adapter: AdapterIdentity,
    pub supported_controls: Vec<ControlKind>,
    pub maximum_isolation: TenantIsolation,
    pub evidence_kinds: Vec<EvidenceKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedControl {
    pub requirement: ControlRequirement,
    pub mechanism: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnforcementPlan {
    pub schema_version: u32,
    pub adapter: AdapterIdentity,
    pub authorization_digest: ContentDigest,
    pub policy_digest: ContentDigest,
    pub workload_digest: ContentDigest,
    pub compiler_version: String,
    pub normalized_workload: NormalizedWorkload,
    pub controls: Vec<PlannedControl>,
    pub adapter_payload: Vec<u8>,
    pub plan_digest: ContentDigest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvidenceStatus {
    Enforced,
    ExternallyAttested,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlEvidence {
    pub requirement: ControlRequirement,
    pub mechanism: String,
    pub status: EvidenceStatus,
    pub evidence_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnforcementEvidence {
    pub adapter: AdapterIdentity,
    pub plan_digest: ContentDigest,
    pub controls: Vec<ControlEvidence>,
    pub runtime_evidence_digest: ContentDigest,
}

pub struct AuthorizedSpawnResult {
    pub spawn: SpawnResult,
    pub evidence: EnforcementEvidence,
    pub rootfs: RootfsLayout,
    pub cgroup_path: InternalPath,
    pub network_allocation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SecureLaunchRequest {
    pub execution_id: String,
    pub container_id: String,
    pub image_layers: Vec<InternalPath>,
    pub container_dir: InternalPath,
    pub run_dir: InternalPath,
    pub workload: NormalizedWorkload,
    pub environment_values: Vec<String>,
    pub capture_output: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanSignature {
    pub key_id: ContentDigest,
    pub signature_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuestSessionBinding {
    pub adapter: AdapterIdentity,
    pub vm_instance_id: String,
    pub boot_nonce: String,
    pub daemon_public_key_base64: String,
    pub guest_public_key_base64: String,
    pub session_digest: ContentDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuestPlanBinding {
    pub session_digest: ContentDigest,
    pub authorization_digest: ContentDigest,
    pub policy_digest: ContentDigest,
    pub workload_digest: ContentDigest,
    pub plan_digest: ContentDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuestPlanEnvelope {
    pub session: GuestSessionBinding,
    pub binding: GuestPlanBinding,
    pub authorization: SignedWorkloadAuthorization,
    pub policy: SecurityPolicyBundle,
    pub plan: EnforcementPlan,
    pub daemon_signature: PlanSignature,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuestEnforcementAttestation {
    pub binding: GuestPlanBinding,
    pub evidence: EnforcementEvidence,
    pub guest_key_id: ContentDigest,
    pub guest_signature_base64: String,
}
```

The adapter payload is opaque to application orchestration but is covered by `plan_digest`.
Compiler and enforcer identity must match. Before allocation, the enforcer derives the effective
spawn configuration from `SecureLaunchRequest`, hashes transient environment values, compares the
result with `plan.normalized_workload`, recomputes its digest, and then recomputes the plan digest. A
mismatch is `PlanIntegrityFailure` before allocation.

`session_digest` hashes the RFC 8785 canonical projection of adapter identity, VM instance ID, boot
nonce, daemon public key, and guest public key, excluding `session_digest` itself. `PlanSignature`
signs RFC 8785 canonical `GuestPlanBinding`, and its `key_id` is the SHA-256 digest of the raw
ephemeral daemon public key in the matching session. At each VM boot, the daemon creates an
ephemeral signing key and boot nonce, supplies its public key through the VM bootstrap channel, and
binds that key to the VM instance and adapter environment digest. The guest creates its own
ephemeral key and proves possession in the handshake. `session_digest` covers the VM instance, boot
nonce, adapter identity, and both public keys.

VM transports carry `GuestPlanEnvelope` only on that VM's authenticated channel. The guest verifies
the original workload authorization, exact plan binding, and daemon signature, then signs
`GuestEnforcementAttestation` with the session-bound guest key. A different VM, boot nonce, adapter,
or prior-boot envelope cannot satisfy the binding.

### Domain Replay And Audit Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizationClaim {
    pub execution_id: String,
    pub principal: AuthorizationPrincipal,
    pub nonce: String,
    pub authorization_digest: ContentDigest,
    pub workload_digest: ContentDigest,
    pub policy_digest: ContentDigest,
    pub expires_at_unix_seconds: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReplayExecutionState {
    Claimed,
    Planned,
    Started,
    Interrupted,
    Completed,
    Denied,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayExecutionRecord {
    pub claim: AuthorizationClaim,
    pub state: ReplayExecutionState,
    pub adapter: Option<AdapterIdentity>,
    pub plan_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SecurityAuditStage {
    Authorization,
    Admission,
    Compilation,
    Enforcement,
    Completion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AdmissionDecision {
    Allowed,
    Denied {
        code: SecurityDenialCode,
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityAuditRecord {
    pub schema_version: u32,
    pub event_id: String,
    pub execution_id: String,
    pub recorded_at_unix_seconds: i64,
    pub stage: SecurityAuditStage,
    pub decision: AdmissionDecision,
    pub authorization_digest: Option<ContentDigest>,
    pub workload_digest: Option<ContentDigest>,
    pub trust_policy_digest: Option<ContentDigest>,
    pub principal: Option<AuthorizationPrincipal>,
    pub signers: Vec<VerifiedSigner>,
    pub policy_digest: Option<ContentDigest>,
    pub adapter: Option<AdapterIdentity>,
    pub plan_digest: Option<ContentDigest>,
    pub evidence_digest: Option<ContentDigest>,
}
```

Replay claims are single-use for new execution. `claim_new` rejects any existing execution ID or
principal+nonce. `resume` accepts only an `Interrupted` record with the same execution ID, principal,
workload digest, policy digest, adapter identity, and plan digest. The replacement authorization may
have a new nonce and authorization digest when the original expired, but it must bind the same
execution, workload, policy, and tenant.

### Domain Errors

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SecurityDenialCode {
    AuthorizationMissing,
    AuthorizationInvalid,
    SignatureRejected,
    AuthorizationExpired,
    AudienceMismatch,
    ReplayDetected,
    PolicyNotFound,
    PolicyDigestMismatch,
    WorkloadDigestMismatch,
    UnsupportedGuarantee,
    AdapterUnqualified,
    OperationDisabled,
    PlanIntegrityFailure,
    AuditUnavailable,
    EnforcementFailure,
}

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("invalid security bundle: {reason}")]
    InvalidBundle { reason: String },
    #[error("invalid workload authorization: {reason}")]
    InvalidAuthorization { reason: String },
    #[error("signature rejected: {reason}")]
    SignatureRejected { reason: String },
    #[error("authorization replay rejected for issuer {issuer} nonce {nonce}")]
    ReplayRejected { issuer: String, nonce: String },
    #[error("policy {name} with digest {digest:?} was not found")]
    PolicyNotFound { name: String, digest: ContentDigest },
    #[error("adapter {adapter} does not support {control:?}")]
    UnsupportedControl { adapter: String, control: ControlKind },
    #[error("security plan integrity failed: {reason}")]
    PlanIntegrity { reason: String },
    #[error("security audit failed: {reason}")]
    Audit { reason: String },
    #[error("security enforcement failed on {adapter}: {reason}")]
    Enforcement { adapter: String, reason: String },
}
```

| Failure Stage              | Public Denial Codes                                                                                             |
| -------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Decode and trust           | `AuthorizationMissing`, `AuthorizationInvalid`, `SignatureRejected`, `AuthorizationExpired`, `AudienceMismatch` |
| Replay and binding         | `ReplayDetected`, `WorkloadDigestMismatch`, `PolicyDigestMismatch`                                              |
| Policy and compiler        | `PolicyNotFound`, `UnsupportedGuarantee`, `AdapterUnqualified`                                                  |
| Operation gating           | `OperationDisabled`                                                                                             |
| Enforcement and durability | `PlanIntegrityFailure`, `AuditUnavailable`, `EnforcementFailure`                                                |

Internal `SecurityError` details are logged and audited; clients receive the stable denial code plus
a redacted reason.

### Domain Ports

```rust
#[async_trait]
pub trait AuthorizationSigner: Send + Sync {
    async fn sign(
        &self,
        manifest: &WorkloadAuthorizationManifest,
    ) -> Result<AuthorizationSignature, SecurityError>;
}

#[async_trait]
pub trait AuthorizationVerifier: Send + Sync {
    async fn verify(
        &self,
        authorization: &SignedWorkloadAuthorization,
        context: &AuthorizationVerificationContext,
    ) -> Result<VerifiedAuthorization, SecurityError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizationVerificationContext {
    pub expected_audience: String,
    pub now_unix_seconds: i64,
}

#[async_trait]
pub trait PolicyBundleResolver: Send + Sync {
    async fn resolve(
        &self,
        reference: &PolicyBundleRef,
    ) -> Result<SecurityPolicyBundle, SecurityError>;

    async fn list(&self) -> Result<Vec<PolicyBundleSummary>, SecurityError>;
}

#[async_trait]
pub trait AuthorizationReplayStore: Send + Sync {
    async fn claim_new(
        &self,
        claim: &AuthorizationClaim,
    ) -> Result<ReplayExecutionRecord, SecurityError>;

    async fn bind_plan(
        &self,
        execution_id: &str,
        adapter: &AdapterIdentity,
        plan_digest: &ContentDigest,
    ) -> Result<(), SecurityError>;

    async fn resume(
        &self,
        execution_id: &str,
        replacement: &VerifiedAuthorization,
    ) -> Result<ReplayExecutionRecord, SecurityError>;

    async fn transition(
        &self,
        execution_id: &str,
        state: ReplayExecutionState,
    ) -> Result<(), SecurityError>;
}

#[async_trait]
pub trait SecurityAuditSink: Send + Sync {
    async fn record(&self, record: &SecurityAuditRecord) -> Result<(), SecurityError>;
}

#[async_trait]
pub trait GuestSecurityChannel: Send + Sync {
    async fn establish(
        &self,
        adapter: &AdapterIdentity,
        guest_image_digest: &ContentDigest,
    ) -> Result<GuestSessionBinding, SecurityError>;

    async fn enforce(
        &self,
        envelope: &GuestPlanEnvelope,
    ) -> Result<GuestEnforcementAttestation, SecurityError>;
}

#[async_trait]
pub trait SecurityProfileCompiler: Send + Sync {
    fn adapter_identity(&self) -> AdapterIdentity;

    async fn capabilities(&self) -> Result<SecurityCapabilities, SecurityError>;

    async fn compile(
        &self,
        authorization: &VerifiedAuthorization,
        policy: &SecurityPolicyBundle,
        workload: &NormalizedWorkload,
    ) -> Result<EnforcementPlan, SecurityError>;
}

#[async_trait]
pub trait SecurityPlanEnforcer: Send + Sync {
    async fn spawn_authorized(
        &self,
        request: &SecureLaunchRequest,
        plan: &EnforcementPlan,
    ) -> Result<AuthorizedSpawnResult, SecurityError>;

    async fn wait_for_exit(
        &self,
        runtime_id: Option<&str>,
        pid: u32,
    ) -> Result<i32, SecurityError>;

    async fn stop(&self, container_id: &str) -> Result<(), SecurityError>;

    async fn cleanup(&self, container_id: &str) -> Result<(), SecurityError>;
}

pub trait SecureContainerRuntime:
    SecurityProfileCompiler + SecurityPlanEnforcer + AsAny + Send + Sync
{
}

pub type DynAuthorizationSigner = Arc<dyn AuthorizationSigner>;
pub type DynAuthorizationVerifier = Arc<dyn AuthorizationVerifier>;
pub type DynPolicyBundleResolver = Arc<dyn PolicyBundleResolver>;
pub type DynAuthorizationReplayStore = Arc<dyn AuthorizationReplayStore>;
pub type DynSecurityAuditSink = Arc<dyn SecurityAuditSink>;
pub type DynGuestSecurityChannel = Arc<dyn GuestSecurityChannel>;
pub type DynSecureContainerRuntime = Arc<dyn SecureContainerRuntime>;
```

The daemon receives only `DynSecureContainerRuntime` for workload allocation, launch, stop, wait,
and cleanup. The secure adapter owns the filesystem, resource limiter, network provider, and raw
runtime it wraps. Raw `DynContainerRuntime` remains available to adapter internals and legacy
non-daemon tests, but no raw spawn-capable port is injected into daemon run handlers.

### Trust Adapter Configuration And Constructors

```rust
// minibox_security

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignerPolicyScope {
    pub allowed_policy_digests: Vec<ContentDigest>,
    pub allowed_profile_classes: Vec<ProfileClass>,
    pub may_authorize_embedded_bundles: bool,
    pub maximum_isolation: TenantIsolation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedLocalKey {
    pub key_id: String,
    pub public_key_path: PathBuf,
    pub scope: SignerPolicyScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedSigstoreIdentity {
    pub issuer: String,
    pub subject: String,
    pub scope: SignerPolicyScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustPolicyConfig {
    pub expected_audience: String,
    pub trusted_local_keys: Vec<TrustedLocalKey>,
    pub trusted_sigstore_identities: Vec<TrustedSigstoreIdentity>,
    pub minimum_valid_signatures: u8,
    pub required_signature_schemes: Vec<SignatureScheme>,
    pub require_transparency_log: bool,
    pub offline_checkpoint_path: Option<PathBuf>,
    pub sigstore_trust_root_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityStorageConfig {
    pub policy_store_dir: PathBuf,
    pub replay_store_path: PathBuf,
    pub audit_log_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityAuthoringConfig {
    pub baseline_policy_name: String,
    pub baseline_policy_digest: ContentDigest,
    pub privileged_policy_name: String,
    pub privileged_policy_digest: ContentDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSignerConfig {
    pub key_id: String,
    pub private_key_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SigstoreSignerConfig {
    pub oidc_issuer: String,
    pub client_id: String,
    pub fulcio_url: String,
    pub rekor_url: String,
}

pub fn strict_decode_signed_authorization(
    value: &serde_json::Value,
) -> Result<SignedWorkloadAuthorization, SecurityError>;

pub fn canonical_authorization_bytes(
    manifest: &WorkloadAuthorizationManifest,
) -> Result<Vec<u8>, SecurityError>;

pub fn workload_digest(workload: &NormalizedWorkload) -> Result<ContentDigest, SecurityError>;

pub fn policy_digest(policy: &SecurityPolicyBundle) -> Result<ContentDigest, SecurityError>;

pub fn authorization_digest(
    manifest: &WorkloadAuthorizationManifest,
) -> Result<ContentDigest, SecurityError>;

pub fn enforcement_plan_digest(
    plan: &EnforcementPlan,
) -> Result<ContentDigest, SecurityError>;

pub fn enforcement_evidence_digest(
    evidence: &EnforcementEvidence,
) -> Result<ContentDigest, SecurityError>;

pub fn execution_manifest_digest(
    manifest: &ExecutionManifest,
) -> Result<ContentDigest, SecurityError>;

pub fn build_authorization_verifier(
    config: TrustPolicyConfig,
) -> Result<DynAuthorizationVerifier, SecurityError>;

pub fn build_local_authorization_signer(
    config: LocalSignerConfig,
) -> Result<DynAuthorizationSigner, SecurityError>;

pub async fn build_sigstore_authorization_signer(
    config: SigstoreSignerConfig,
) -> Result<DynAuthorizationSigner, SecurityError>;

pub fn open_policy_bundle_resolver(
    config: &SecurityStorageConfig,
) -> Result<DynPolicyBundleResolver, SecurityError>;

pub fn open_authorization_replay_store(
    config: &SecurityStorageConfig,
) -> Result<DynAuthorizationReplayStore, SecurityError>;

pub fn open_security_audit_sink(
    config: &SecurityStorageConfig,
) -> Result<DynSecurityAuditSink, SecurityError>;
```

Digest projections are fixed protocol contracts:

- `workload_digest` hashes RFC 8785 canonical `NormalizedWorkload`.
- `policy_digest` hashes RFC 8785 canonical `SecurityPolicyBundle`.
- `authorization_digest` hashes only RFC 8785 canonical `WorkloadAuthorizationManifest`; signature
  order and additional valid signatures do not change authorization identity.
- `enforcement_plan_digest` hashes every `EnforcementPlan` field except `plan_digest`, including the
  normalized workload and opaque adapter payload.
- `enforcement_evidence_digest` hashes every `EnforcementEvidence` field except
  `runtime_evidence_digest`.
- `execution_manifest_digest` hashes the stable execution-manifest projection, including its
  security record and excluding `execution_digest`, timestamps, and filesystem paths.

Every struct and enum in the transitive signed payload uses `#[serde(deny_unknown_fields)]`.
`strict_decode_signed_authorization` retains the protocol value as raw JSON until this recursive
typed decode succeeds. Unknown nested claims, controls, references, or digest fields are rejected
rather than discarded. The sole opaque exception is `AuthorizationSignature::Sigstore.bundle`,
which is passed intact to the Sigstore verifier and validated against the supported Sigstore bundle
schema; Minibox does not reinterpret or discard fields inside it.
The verifier accepts signatures from distinct trusted identities until
`minimum_valid_signatures` and every `required_signature_schemes` entry are satisfied; all accepted
identities are retained in `VerifiedAuthorization.signers`.
Startup rejects a zero signature threshold, a threshold greater than the number of distinct trusted
identities, a required scheme with no trusted identity, duplicate key IDs, duplicate Sigstore
issuer/subject pairs, and signer scopes that cannot satisfy their declared profile classes.

The composition root adds this exact fail-closed configuration:

```rust
// miniboxd::config

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DaemonSecurityConfig {
    pub trust_policy_path: PathBuf,
    pub policy_store_dir: PathBuf,
    pub replay_store_path: PathBuf,
    pub audit_log_path: PathBuf,
}
```

`DaemonConfig` gains a required `security: DaemonSecurityConfig` field. Missing or invalid security
configuration is a startup error rather than a default-value fallback.

The baseline and privileged aliases live in trusted `mbx` authoring configuration, not in daemon
admission. Omitting `--profile` while authoring selects the pinned baseline name+digest;
`--privileged` selects the pinned privileged name+digest. The resulting authorization still carries
an explicit policy reference and signature, so the daemon never manufactures or substitutes policy.

Private signing keys are read only by `mbx` signing commands. MCP and Crux never receive signer
ports or key paths.

### Daemon Dependency Injection

```rust
// minibox::daemon::handler

#[derive(Clone)]
pub struct SecurityDeps {
    pub verifier: DynAuthorizationVerifier,
    pub policy_resolver: DynPolicyBundleResolver,
    pub replay_store: DynAuthorizationReplayStore,
    pub audit_sink: DynSecurityAuditSink,
    pub runtime: DynSecureContainerRuntime,
    pub expected_audience: String,
}

#[derive(Clone)]
pub struct PtyDeps {
    pub pty_sessions: SharedPtyRegistry,
}

#[derive(Clone)]
pub struct ImageMutationDeps {
    pub image_pusher: Option<DynImagePusher>,
    pub commit_adapter: Option<DynContainerCommitter>,
}

#[derive(Clone)]
pub struct HandlerDependencies {
    pub image: ImageDeps,
    pub pty: PtyDeps,
    pub image_mutation: ImageMutationDeps,
    pub events: EventDeps,
    pub security: SecurityDeps,
    pub checkpoint: DynVmCheckpoint,
}
```

`ContainerPolicy`, `PolicyOverride`, and optional `execution_policy` are removed from execution
authority. `LifecycleDeps` is removed from admitted handler dependency injection; its filesystem,
limiter, network, and raw-runtime implementations move behind each secure-runtime wrapper.
`ExecDeps.exec_runtime` and `BuildDeps.image_builder` are also removed, leaving no raw process-spawn
authority reachable from daemon handlers. MCP policy remains an independent, stricter caller
boundary.

The run path has one pure normalization boundary:

```rust
// minibox::daemon::security
pub(crate) fn normalize_run_request(
    params: &RunParams,
    resolved_image_digest: ContentDigest,
    effective_command: Vec<String>,
    effective_user: Option<String>,
) -> Result<NormalizedWorkload, SecurityError>;
```

| Existing Type | Exact Fields Added To `RunParams`                                                                                                                                                                                                                                                                                                        |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `RunParams`   | `pids_max: Option<u64>`, `io_max_bytes_per_sec: Option<u64>`, `tty: bool`, `entrypoint: Option<String>`, `user: Option<String>`, `auto_remove: bool`, `urgency: Option<slashcrux::Urgency>`, `execution_context: Option<slashcrux::ExecutionContext>`, `authorization: Option<serde_json::Value>`, `resume_execution_id: Option<String>` |

The daemon resolves image defaults once, normalizes every effective security-relevant field, and
compares the result byte-for-byte with the signed normalized workload before compilation.
`SecureLaunchRequest` then carries that same value into the enforcer; no later layer reconstructs a
different workload from partial parameters.

### Execution Manifest Extension

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionSecurityStatus {
    Planned,
    Enforced,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionSecurityRecord {
    pub execution_id: String,
    pub authorization_digest: ContentDigest,
    pub workload_digest: ContentDigest,
    pub policy_digest: ContentDigest,
    pub trust_policy_digest: ContentDigest,
    pub principal: AuthorizationPrincipal,
    pub signers: Vec<VerifiedSigner>,
    pub adapter: AdapterIdentity,
    pub plan_digest: ContentDigest,
    pub status: ExecutionSecurityStatus,
    pub evidence: Option<EnforcementEvidence>,
}
```

| Existing Type       | Exact Field Added                                                                                          |
| ------------------- | ---------------------------------------------------------------------------------------------------------- |
| `ExecutionManifest` | `#[serde(default, skip_serializing_if = "Option::is_none")] pub security: Option<ExecutionSecurityRecord>` |
| `ExecutionManifest` | `#[serde(default, skip_serializing_if = "Option::is_none")] pub execution_digest: Option<ContentDigest>`   |

Old manifests remain readable through the optional field. New runs reject completion without a
security record. Before launch, the daemon atomically persists `Planned` with no evidence; after the
enforcement handshake it atomically replaces that record with `Enforced` and evidence, or `Failed`
on cleanup. The existing workload digest remains the normalized workload identity and excludes
security evidence; the new execution digest binds the current security record without
self-reference.

Status invariants are strict: `Planned` and `Failed` require `evidence: None`; `Enforced` requires
`Some` evidence whose digest, adapter, and plan binding verify. At daemon startup, a `Planned`
manifest is never promoted from process existence alone. Any associated live process or guest is
terminated and cleaned up, then the manifest and replay record become `Failed`; retry requires a new
execution authorization.

### Protocol Additions

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityPreflightReport {
    pub decision: AdmissionDecision,
    pub authorization_digest: Option<ContentDigest>,
    pub policy_digest: Option<ContentDigest>,
    pub workload_digest: Option<ContentDigest>,
    pub adapter: Option<AdapterIdentity>,
    pub plan_digest: Option<ContentDigest>,
    pub controls: Vec<PlannedControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityPreflightInput {
    pub authorization: serde_json::Value,
    pub intended_workload: NormalizedWorkload,
}
```

| Existing Type                | Exact Additions                                                                                                                                                                                                                  |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DaemonRequest::Run`         | `pids_max: Option<u64>`, `io_max_bytes_per_sec: Option<u64>`, `authorization: Option<serde_json::Value>`, `resume_execution_id: Option<String>`; each uses additive `#[serde(default, skip_serializing_if = "Option::is_none")]` |
| `DaemonRequest::RunPipeline` | `authorization: Option<serde_json::Value>` with additive `#[serde(default, skip_serializing_if = "Option::is_none")]`                                                                                                            |
| `DaemonRequest`              | `PreflightSecurity { input: SecurityPreflightInput }`                                                                                                                                                                            |
| `DaemonRequest`              | `ListSecurityPolicies`                                                                                                                                                                                                           |
| `DaemonRequest`              | `InspectSecurityPolicy { name: String, digest: ContentDigest }`                                                                                                                                                                  |
| `DaemonResponse`             | `SecurityPreflight { report: SecurityPreflightReport }`                                                                                                                                                                          |
| `DaemonResponse`             | `SecurityPolicies { policies: Vec<PolicyBundleSummary> }`                                                                                                                                                                        |
| `DaemonResponse`             | `SecurityPolicy { bundle: SecurityPolicyBundle }`                                                                                                                                                                                |

The protocol keeps authorization as raw JSON until strict recursive validation completes. The field
is additive for deserialization compatibility, but `None` yields `AuthorizationMissing` once
admission is enabled. Existing `privileged: bool` is deprecated and must agree with the signed
bundle class; it cannot select privilege by itself. `resume_execution_id` must equal the signed
claim's execution ID and refer to an interrupted replay record.

### Client Surface Changes

```rust
// mbx::commands::security
pub async fn sign_local(
    manifest_path: &Path,
    output_path: &Path,
    config: LocalSignerConfig,
) -> anyhow::Result<()>;

pub async fn sign_sigstore(
    manifest_path: &Path,
    output_path: &Path,
    config: SigstoreSignerConfig,
) -> anyhow::Result<()>;

pub async fn preflight(
    authorization_path: &Path,
    intended_workload: NormalizedWorkload,
    socket_path: &Path,
) -> anyhow::Result<SecurityPreflightReport>;

pub async fn inspect(
    name: String,
    digest: ContentDigest,
    socket_path: &Path,
) -> anyhow::Result<SecurityPolicyBundle>;
```

| Existing Type                   | Exact Fields Added                                                                               |
| ------------------------------- | ------------------------------------------------------------------------------------------------ |
| `mbx::commands::run::RunOpts`   | `authorization_path: PathBuf`, `pids_max: Option<u64>`, `io_max_bytes_per_sec: Option<u64>`      |
| `mcp::types::RunContainerInput` | `authorization: serde_json::Value`, `pids_max: Option<u64>`, `io_max_bytes_per_sec: Option<u64>` |

The Crux `minibox::container::run` input adds the same `authorization` object. Neither MCP nor Crux
adds signing operations. CLI `--privileged` becomes shorthand used while creating a privileged
authorization; at execution time the signed profile class is authoritative.

Every `container-run` entry in `WorkflowDef.steps` carries its own signed authorization in the
step-specific `config` object. `RunPipeline` carries the authorization for its ephemeral pipeline
container. A parent workflow or pipeline signature cannot implicitly authorize a different nested
workload.

## Data Flow

1. **Authoring**: An operator or trusted workload issuer resolves an immutable image digest,
   normalizes the workload request, selects an immutable policy bundle, computes canonical
   workload/policy digests, and signs the authorization with Ed25519 or Sigstore.
2. **Transport**: CLI, MCP, or Crux sends the unchanged signed authorization beside the run request;
   client-side policy may reject but cannot widen it.
3. **Initial verification**: The daemon validates schema, canonicalization algorithm, signature,
   signer scope, audience, validity window, and request fields before image or workload allocation.
4. **Image binding**: The daemon resolves the image identity and compares it with the signed immutable
   digest before rootfs creation.
5. **Replay claim**: The signed claims supply the execution ID; the daemon atomically claims
   execution ID plus principal+nonce+digests. A new run rejects existing claims, while an explicit
   resume request must match an interrupted record.
6. **Policy resolution**: Embedded bundles are canonicalized directly; stored bundles are loaded by
   digest. Either path must equal the signed policy digest.
7. **Compilation**: The selected secure runtime probes current host/guest capabilities and compiles
   every control into one adapter-bound plan. Any unsupported control rejects admission.
   `PreflightSecurity` first compares its intended workload with the signed workload, then runs
   through this step without claiming the nonce or allocating resources. The actual run derives the
   effective workload again and repeats verification and compilation because preflight is not an
   authority grant.
8. **Audit-before-launch**: The daemon durably records the allow/deny decision and atomically writes
   the planned execution manifest. Failure to persist either rejects execution.
9. **Enforcement**: The same secure runtime verifies the plan digest, creates resources, applies all
   controls, and returns enforcement evidence. Partial failure tears resources down and records a
   failed outcome without retrying a weaker plan.
10. **Persistence**: The daemon finalizes authorization, policy, plan, adapter, and evidence digests
    in the execution manifest and replay store without persisting environment values or private
    keys.
11. **Replay**: Crux traces reference immutable digests. A new execution requires fresh authorization;
    crash resumption is allowed only for the exact claimed execution and plan inputs while the
    original authorization remains valid, or after a replacement authorization binds the same
    execution, principal, workload, and policy. Adapter identity and plan digest must remain exact;
    an adapter/compiler upgrade requires a new execution authorization rather than crash resumption.

## Hexagonal Boundaries

- **Authorization signer port**: `AuthorizationSigner` in `minibox-domain`; Ed25519 and Sigstore
  implementations in `minibox-security` and used only by trusted CLI workflows.
- **Authorization verifier port**: `AuthorizationVerifier` in `minibox-domain`; composite trust
  implementation in `minibox-security`.
- **Policy resolver port**: `PolicyBundleResolver` in `minibox-domain`; immutable filesystem adapter
  in `minibox-security`.
- **Replay port**: `AuthorizationReplayStore` in `minibox-domain`; atomic disk adapter in
  `minibox-security`.
- **Audit port**: `SecurityAuditSink` in `minibox-domain`; durable JSONL adapter in
  `minibox-security`. Existing non-blocking `EventSink` is not used for security authority.
- **Guest channel port**: `GuestSecurityChannel` in `minibox-domain`; VM-specific authenticated
  vsock/process-channel implementations live with SmolVM, krun, VZ, and dedicated Colima adapters.
- **Compilation port**: `SecurityProfileCompiler` in `minibox-domain`; one implementation per
  runtime adapter.
- **Enforcement port**: `SecurityPlanEnforcer` in `minibox-domain`; paired with its compiler through
  `SecureContainerRuntime` so a plan cannot be sent to a different runtime type.
- **Application service**: daemon admission in `minibox`; it coordinates ports but contains no
  cryptography or platform syscalls.

## Adapter Enforcement Model

### Secondary Execution Paths

- `RunPipeline` and each workflow `container-run` step carry a workload-specific authorization and
  use the same secure runtime.
- `mbx sandbox` is a client wrapper over authorized `Run`; it has no independent execution path.
- `Update { restart: true }` and `RestoreSnapshot` reject secured containers until their explicit
  replacement/resume authorization protocols are implemented.
- `Build`, host lifecycle hooks, and `Exec` are disabled in hostile-multitenant deployments. An exec
  implementation may be enabled later only when the new process inherits or independently enforces
  the stored plan and the caller is authorized by the trusted scheduler.
- Stop, wait, and cleanup use the existing secured container's runtime identity and do not create a
  new workload. They remain available only to the trusted scheduler control plane.

### Native Linux

- Add `CLONE_NEWUSER` and a parent/child synchronization channel. The parent writes `setgroups`,
  `uid_map`, and `gid_map` before the child proceeds.
- Apply capability bounding, permitted, effective, inheritable, and ambient sets from the compiled
  ceiling. Standard profiles default to an empty set unless explicitly allowed.
- Set `no_new_privs`, install a general default-deny seccomp plan, and compose the existing mount
  immutability ratchet before `execve`.
- Rootless profiles require delegated cgroups, a rootless-capable filesystem path, and a supported
  network mechanism. Missing delegation or helper capability is a compile/admission failure, not a
  warning.
- A child-init status channel confirms user mapping, capability, seccomp, mount, and identity setup
  before the parent reports successful spawn.

### GKE And proot

proot is treated only as path translation. A hostile profile compiles only when the outer pod
environment proves non-root execution, capability drop, seccomp, resources, network policy, and a
qualifying sandbox boundary. Missing or unverifiable outer evidence rejects the profile.

### SmolVM, krun, And VZ

The VM boundary supplies host isolation, while a versioned guest agent compiles/applies in-guest
identity, capability, syscall, filesystem, network, and resource controls. Startup performs a
capability handshake and returns guest evidence bound to the image, guest version, policy, workload,
and plan digests. An old or unreachable guest rejects the run.

### Colima And Docker Desktop

A shared developer VM cannot qualify for hostile multi-tenancy. These adapters either provision a
dedicated, policy-bound VM with the same guest evidence contract or reject hostile profiles. Weaker
single-tenant profiles remain possible only when explicitly signed for that isolation target.

### Windows HCS And WSL2

HCS maps hostile profiles to Hyper-V-isolated containers and returns HCS/guest evidence. WSL2 and
process-isolated Windows containers reject hostile profiles unless a later implementation proves an
equivalent dedicated boundary. Named Pipe authentication is part of Windows admission and cannot be
deferred once Windows run support is enabled.

### Library-Only And Stub Adapters

`VfRuntime`, current HCS stubs, and other non-selectable runtime implementations return a structured
`AdapterUnqualified` denial. They do not advertise security controls from coarse
`RuntimeCapabilities` booleans.

## Failure, Cleanup, And Audit Semantics

```text
new authorization -> Claimed -> Planned -> Started -> Completed
                         |          |          |
                         v          v          +-> Interrupted -> explicit resume -> Started
                       Denied     Failed       +-> Failed
```

Only `Interrupted` is resumable. `Completed`, `Denied`, and `Failed` are terminal; retrying them
requires a new execution ID and authorization.

- Signature, audience, freshness, replay, workload, bundle, and compilation failures occur before
  rootfs, network, cgroup, VM, or process creation.
- Image resolution may occur after initial signature verification but before enforcement compilation;
  a digest mismatch stops before rootfs creation.
- Allocation after compilation is transactional. Enforcement failure terminates the child/guest,
  tears down network and resources, removes incomplete state, marks replay state failed, and writes a
  durable failure record.
- There is no retry under a different adapter, weaker profile, reduced control set, or permissive
  seccomp/capability mode.
- Invalid trust configuration, unreadable trust roots, unsupported configured signature schemes,
  and unavailable durable audit/replay stores prevent daemon startup.
- A completion-audit failure after launch marks the security subsystem unhealthy, stops a still-live
  workload when possible, and blocks new runs until durable audit storage recovers.
- Audit records contain identities and content digests, not private keys, plaintext environment
  values, OIDC tokens, or raw guest secrets.

## Verification Strategy

### Contract And Cryptography

- RFC 8785 interoperability fixtures pin canonical bytes across field orderings.
- SHA-256 fixtures pin workload, policy, authorization, plan, and evidence digests.
- Ed25519 fixtures cover valid, malformed, wrong-key, revoked-key, and signer-scope cases.
- Sigstore fixtures cover identity, issuer, certificate validity, transparency inclusion, offline
  checkpoint, malformed bundle, and unavailable-network fail-closed behavior.
- Property tests cover round-trips, unknown fields, duplicate controls, contradictory controls,
  ordering normalization, time windows, and digest mismatch.

### Admission And Replay

- Unit tests prove every denial occurs before allocator/adaptor calls.
- Concurrent nonce claims prove exactly one new execution wins.
- Crash fixtures prove exact execution resumption and reject changed execution IDs, principals,
  workloads, policies, adapters, or plan digests while allowing a fresh authorization digest that
  binds the same approved execution.
- Failure-injection tests cover policy store, replay store, audit sink, compiler, enforcer, and
  cleanup failures.
- Mutation tests remove one verification/control step at a time and require a failing test.

### Adapter Conformance

Every adapter runs the same semantic matrix and must either produce valid evidence or reject before
allocation. Mock-only conformance is insufficient for qualification.

- Native Linux executes real userns, capability, seccomp, cgroup, filesystem, process, and network
  attacks on a supported root/rootless test host.
- VM adapters use real guest-agent handshakes and tampered-evidence tests.
- GKE validates outer sandbox evidence in a real cluster configuration.
- Colima/Docker Desktop prove shared-VM rejection and dedicated-VM isolation separately.
- Windows validates HCS Hyper-V isolation and WSL2 rejection on Windows CI.

### Required Gates

- `cargo fmt --all`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run --workspace`
- `cargo xtask architecture`
- `cargo xtask protocol-drift`
- `cargo xtask musl-check`
- Real Linux/root and Linux/rootless security suites
- Per-adapter guest/cluster/Windows qualification suites

## External Dependencies And Features

`minibox-security` isolates all new security implementation dependencies:

- `serde_json_canonicalizer` for RFC 8785 canonical JSON.
- `ed25519-dalek` for local Ed25519 signing and verification, with private-key zeroization enabled.
- `sigstore` with minimal Rustls, bundle, verification, signing, and trust-root features; default
  features remain disabled.

The crate exposes `local-signing` and `sigstore` features for build composition, but release builds
of `mbx` and `miniboxd` enable both. A build lacking a signature implementation rejects trust config
or authorization that requests it; it never falls back to another scheme.

## Integration And Compatibility

- `DaemonRequest::Run.authorization` is wire-additive with `#[serde(default)]`, but the semantic
  change is intentionally breaking because missing authorization is denied.
- `ExecutionManifest.security` is additive and optional only for reading historical manifests.
- `ContainerRuntime` remains source-compatible; daemon launch moves to the new required secure port.
- Every handler-dependency constructor and adapter-suite composition site changes in the admission
  slice.
- Protocol snapshots, property generators, benchmarks, VZ guest requests, MCP schemas, Crux handler
  declarations, and CLI tests must update together.
- The workspace architecture gate must recognize `minibox-security` as an outward adapter depending
  only on `minibox-domain` and third-party libraries.
- This is a semver-significant behavior and public-trait change for pre-1.0 published crates; release
  notes must call out mandatory authorization and adapter qualification.

## Out Of Scope

- Best-effort enforcement, silent guarantee downgrade, or unsigned compatibility modes.
- Caller-supplied raw seccomp BPF, HCS JSON, VM commands, or other platform-native policy payloads.
- A centralized policy-authority service, organization-wide key management, or hosted signing API.
- Direct tenant access to the daemon control plane and tenant ownership authorization for lifecycle
  APIs; qualified deployments expose the daemon only to trusted schedulers.
- Secure Dockerfile/build execution and new exec processes under hostile profiles. Those operations
  remain disabled in a hostile-multitenant deployment until they gain equivalent authorization and
  inherited-plan enforcement designs.
- Hardware-backed confidential computing, remote TEE attestation, and side-channel mitigation.
- Protection from a compromised host kernel, hypervisor, daemon, trusted guest image, or signing key.
- Automatic placement across remote Minibox hosts; this design qualifies the selected local adapter.
- Billing, quotas across multiple hosts, and tenant identity provisioning outside signed claims.
- Rewriting the engineering roadmap; documentation updates follow implementation and qualification.

## Risk

- [x] Breaking API/behavior: mandatory authorization and secure-runtime injection change every run
      path and adapter suite.
- [x] New external dependencies: `serde_json_canonicalizer`, `ed25519-dalek`, and `sigstore`, isolated
      in `minibox-security`.
- [x] Dependency maturity: the Rust `sigstore` API is experimental, so it remains behind the domain
      verifier/signer ports and is pinned with interoperability fixtures.
- [x] Feature flags required: local and Sigstore implementations are selectable at build time, but
      release binaries enable both and reject unavailable configured schemes.
- [x] Serialization migration: four new persisted schemas and protocol additions require golden and
      property fixtures.
- [x] Cross-platform verification: production qualification depends on Linux, VM, GKE, and Windows
      environments unavailable to a macOS-only unit-test run.
- [x] Availability impact: unqualified adapters and unsigned existing clients stop running by
      design.
- [x] Security claim risk: hostile-multitenant support is published per adapter/profile/host matrix,
      never as an unconditional project-wide label.

## Acceptance Criteria

1. Every public run surface requires a valid signed authorization and returns the same structured
   denial codes for equivalent failures.
2. Local Ed25519 and Sigstore identities verify against operator-configured scopes and fail closed on
   expiry, revocation, audience mismatch, unavailable required trust data, or transparency failure.
3. Profile-class and hostile-multitenant baseline validation rejects signed bundles that omit or
   weaken mandatory controls.
4. Policy, workload, authorization, plan, evidence, and execution digests use explicit canonical
   projections and reproduce from independent fixtures.
5. The secure runtime launches only from the normalized workload embedded in the verified plan;
   request, image, environment, adapter, and plan mismatches fail before allocation.
6. Replay storage admits one new execution per execution ID and principal+nonce and resumes only the
   exact interrupted execution with an exact adapter and plan binding.
7. Every selectable adapter supplies one paired compiler/enforcer object or is unavailable for run.
8. Every compiled control has matching authenticated enforcement evidence; missing or invalid
   evidence fails the run.
9. Standard profiles cannot obtain privileged capabilities through legacy flags, pipeline overrides,
   client policy, guest-version mismatch, or adapter fallback.
10. Update restart, snapshot restore, build, exec, sandbox, pipeline, and workflow paths either route
    through authorized execution or reject under hostile-multitenant configuration.
11. Native Linux proves default capability drop, user namespace mapping, general seccomp, rootless
    resource behavior, and cleanup with real kernel tests.
12. VM, managed, and Windows adapters either pass their real-environment matrix or reject the hostile
    profile before allocation.
13. Execution manifests and audit records bind principal, signers, trust policy, authorization,
    policy, workload, adapter,
    plan, and enforcement evidence digests without storing secrets.
14. All required Cargo and xtask gates pass for each delivery slice.
15. Production documentation lists only verified adapter/profile/host combinations as
    hostile-multitenant capable.
