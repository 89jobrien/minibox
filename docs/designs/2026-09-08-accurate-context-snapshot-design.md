---
source_sha: 304cb7f8b7a583efd4977e055eb77030256a0324
sources:
  - Cargo.toml
  - CLAUDE.md
  - crates/mbx/Cargo.toml
  - crates/mbx/src/commands/doctor.rs
  - crates/minibox-core/Cargo.toml
  - crates/miniboxd/src/adapter_registry.rs
  - docs/core/FEATURE_MATRIX.mbx.md
  - docs/core/XTASK_CLI.mbx.md
  - docs/plans/2026-08-25-xtask-info-context-enhancements.md
  - xtask/Cargo.toml
  - xtask/schema/cli.schema.json
  - xtask/src/architecture.rs
  - xtask/src/collect_metrics.rs
  - xtask/src/context.rs
  - xtask/src/docs_audit.rs
  - xtask/src/main.rs
  - xtask/src/test_image.rs
  - xtask/src/xconfig.rs
  - xtask/xconfig.toml
generated: 2026-09-08
---

# Design: Accurate Context Snapshot V3

## Goal

Replace the ambiguous v2 repository snapshot with a v3 evidence ledger that separates declared,
observed, and validated facts so every value is traceable and unavailable facts are never
presented as known.

## Approved Approach

Use the approved **Evidence-Ledger Snapshot** approach: collect repository declarations and live
observations independently, reconcile them through explicit validation states, and retain
provenance and diagnostics in the emitted snapshot.

## Context Map

### Files to Modify

| File                                  | Purpose                                | Changes Needed                                                                                   |
| ------------------------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `xtask/src/context.rs`                | Context command orchestration          | Replace v2 collection with v3 orchestration, strict handling, persistence, and child modules     |
| `xtask/src/context/model.rs`          | Serialized v3 model                    | Add evidence-backed snapshot, fact, diagnostic, adapter, test, ownership, and task types         |
| `xtask/src/context/manifest.rs`       | Canonical context configuration        | Load and validate adapter declarations and named validation profiles                             |
| `xtask/src/context/collect.rs`        | External collection adapters           | Collect Git, environment, Cargo metadata, tracked files, and nextest JSON through testable ports |
| `xtask/src/context/test_inventory.rs` | Declared and executable test inventory | Parse Rust declarations and nextest JSON without conflating their counts                         |
| `xtask/src/context/output.rs`         | Snapshot output boundary               | Validate v3 JSON, persist snapshots/evidence, and render generated adapter documentation         |
| `xtask/src/main.rs`                   | CLI dispatch                           | Parse `--strict`, `--validate-all`, and `--evidence-dir`; add `docs sync-adapters`               |
| `xtask/context.toml`                  | Canonical typed manifest               | Declare adapter maturity/capabilities and bounded target/feature validation profiles             |
| `xtask/Cargo.toml`                    | Collector dependencies                 | Add typed Cargo metadata, Rust syntax, and JSON Schema validation libraries                      |
| `xtask/schema/cli.schema.json`        | Machine-readable CLI/output contract   | Replace the v2 output definition with the complete v3 schema and new flags                       |
| `docs/core/XTASK_CLI.mbx.md`          | Human-readable command contract        | Document v3 semantics, flags, validation states, cache behavior, and exit behavior               |
| `docs/core/FEATURE_MATRIX.mbx.md`     | Adapter capability reference           | Replace the hand-maintained adapter table with a manifest-generated marked block                 |
| `xtask/src/docs_audit.rs`             | Documentation drift gate               | Validate the generated adapter block against `xtask/context.toml`                                |

### Dependencies And Consumers

| File                                      | Relationship                                                                                                       |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `crates/miniboxd/src/adapter_registry.rs` | Observed runtime registry compared with canonical adapter IDs, platforms, and defaults; no runtime behavior change |
| `crates/mbx/src/commands/doctor.rs`       | Existing duplicate adapter declarations become a reported drift source; changing doctor behavior is out of scope   |
| `xtask/src/collect_metrics.rs`            | Analogous collector whose annotation count remains distinct from v3 declared/executable test counts                |
| `xtask/src/architecture.rs`               | Reference for typed Cargo metadata collection and exact dependency-kind handling                                   |
| `xtask/src/test_image.rs`                 | Reference for consuming Cargo JSON compiler messages                                                               |
| `.github/workflows/*.yml`                 | Native CI jobs produce profile evidence and an aggregation job merges same-commit artifacts                        |

Workflow changes are integration work derived from this design. They must use the repository's
workflow-editing procedure and must not modify unrelated dirty workflow changes.

### Existing Test Coverage

| Test                                                                           | Current Coverage      | Required V3 Change                                                          |
| ------------------------------------------------------------------------------ | --------------------- | --------------------------------------------------------------------------- |
| `xtask/src/context.rs::context_snapshot_includes_context_map`                  | Presence of v2 keys   | Replace with full v3 schema validation                                      |
| `xtask/src/context.rs::crate_assignments_are_sorted_and_stable`                | Line-count ranking    | Replace with exact Cargo package ownership and deterministic ordering cases |
| `xtask/src/context.rs::file_assignments_cover_xtask_info_context_surface`      | Four hard-coded paths | Replace with tracked-file ownership and changed-file focus cases            |
| `xtask/src/context.rs::task_slices_define_expected_dependency_graph`           | Fixed four-task graph | Replace with executed collector DAG and status cases                        |
| `xtask/src/context.rs::save_mode_persists_context_map_in_snapshot_and_history` | v2 persistence        | Extend to content-keyed profile evidence and v3 history                     |
| `xtask/src/main.rs::info_context_args_accept_only_optional_save`               | `--save` parsing      | Cover every valid flag combination and conflicting/unknown arguments        |

No current test covers malformed Cargo metadata, malformed nextest JSON, unavailable targets,
dirty-worktree identity, adapter-manifest drift, schema rejection, or strict-mode exit behavior.

### Reference Patterns

| File                          | Pattern to Follow                                                       |
| ----------------------------- | ----------------------------------------------------------------------- |
| `xtask/src/xconfig.rs`        | Typed TOML loading with path-specific `anyhow::Context`                 |
| `xtask/src/architecture.rs`   | Typed Cargo metadata and dependency-kind inspection                     |
| `xtask/src/test_image.rs`     | Line-oriented Cargo JSON collection with explicit process-status checks |
| `xtask/src/docs_audit.rs`     | Cross-checking source declarations against documentation facts          |
| `xtask/src/protocol_drift.rs` | Deterministic serialized records and drift diagnostics                  |

### Context-Map Risks

- The v3 JSON shape intentionally breaks v2 consumers; v2 compatibility mode is explicitly
  rejected.
- The current working tree contains unrelated modifications. Implementation must preserve them
  and isolate edits to the files named by the eventual plan.
- Test discovery is platform- and feature-sensitive. A static declaration must never be labeled
  executable without current profile evidence.
- Adapter maturity and capability are product declarations, not facts that can be proved solely
  from source presence. Validation can prove registry/docs/test consistency, not runtime quality.
- Cross-target validation may be unavailable locally. This is represented as data rather than
  converted to zero tests or a successful validation.

## Crate Ownership

- **Owner crate**: `xtask` -- repository introspection, process execution, schema validation,
  documentation synchronization, and derived artifact persistence already live here.
- **Observed product crate**: `miniboxd` -- its adapter registry is evidence consumed by the
  validator, but it does not depend on `xtask` and gains no new runtime dependency.
- **Affected product crate**: none. The design changes tooling contracts and generated
  documentation, not daemon, CLI, protocol, or adapter behavior.

No new workspace crate is required. `xtask` remains a leaf binary, so the design introduces no
circular dependency or outward dependency from a domain ring.

## Canonical Manifest

`xtask/context.toml` is the declared source of truth for facts that Cargo cannot express:

- stable adapter IDs;
- maturity (`production`, `experimental`, `blocked`, or `stub`);
- supported platforms and default/fallback roles;
- capability support (`yes`, `limited`, `blocked`, or `no`);
- bounded validation profiles with target triple, feature selection, target selection, and CI
  requirement.

The manifest does not claim compile-time availability or runtime health. Those are observations
from the adapter registry, Cargo/nextest, and CI evidence. The profile list is curated and
versioned; the collector does not attempt the complete Cargo feature power set.

## Internal API

The design adds no public workspace-library API. Items are crate-internal to the `xtask` binary;
signatures are specified to keep module boundaries stable and testable.

### Command Options

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ContextOptions {
    pub(super) save: bool,
    pub(super) strict: bool,
    pub(super) validate_all: bool,
    pub(super) evidence_dir: Option<std::path::PathBuf>,
}

pub(super) fn context(
    runner: &impl CommandRunner,
    repository: &impl RepositoryReader,
    root: &std::path::Path,
    options: &ContextOptions,
) -> anyhow::Result<()>;

fn parse_info_context_args(rest: &[String]) -> anyhow::Result<ContextOptions>;
```

`--evidence-dir <path>` reads additional profile evidence, primarily artifacts downloaded by a CI
aggregation job. It does not make stale evidence current; all cache keys are validated before
merge.

### Ports And System Adapters

```rust
pub(super) trait CommandRunner {
    fn run(&self, command: &CommandSpec) -> anyhow::Result<CommandOutput>;
}

pub(super) trait RepositoryReader {
    fn read_utf8(&self, path: &std::path::Path) -> anyhow::Result<String>;
    fn tracked_files(&self, root: &std::path::Path) -> anyhow::Result<Vec<RepositoryPath>>;
    fn file_metrics(&self, path: &std::path::Path) -> anyhow::Result<FileMetrics>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommandSpec {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) current_dir: std::path::PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommandOutput {
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SystemCommandRunner;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SystemRepositoryReader;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub(super) struct RepositoryPath(String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileMetrics {
    pub(super) physical_lines: usize,
    pub(super) non_empty_lines: usize,
    pub(super) content_sha256: String,
}
```

Only program names and argument arrays enter evidence. Environment values and raw stderr are not
persisted, preventing secrets or machine-specific diagnostics from leaking into snapshots.

### Evidence And Validation Types

```rust
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub(super) struct EvidenceId(String);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EvidenceKind {
    CargoMetadata,
    Git,
    Manifest,
    Nextest,
    RustSource,
    ToolVersion,
    TrackedFile,
    CiArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EvidenceStatus {
    Collected,
    Failed,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Evidence {
    pub(super) kind: EvidenceKind,
    pub(super) locator: String,
    pub(super) collected_at: String,
    pub(super) content_sha256: Option<String>,
    pub(super) profile_id: Option<String>,
    pub(super) status: EvidenceStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ValidationState {
    Match,
    Mismatch,
    DeclaredOnly,
    ObservedOnly,
    Unavailable,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Validation {
    pub(super) state: ValidationState,
    pub(super) reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Fact<T> {
    pub(super) declared: Option<T>,
    pub(super) observed: Option<T>,
    pub(super) validation: Validation,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct ContextDiagnostic {
    pub(super) code: String,
    pub(super) severity: DiagnosticSeverity,
    pub(super) message: String,
    pub(super) profile_id: Option<String>,
    pub(super) evidence_ids: Vec<EvidenceId>,
}
```

### Snapshot Types

```rust
#[derive(Debug, serde::Serialize)]
pub(super) struct ContextSnapshot {
    pub(super) snapshot_version: u32,
    pub(super) identity: RepositoryIdentity,
    pub(super) environment: EnvironmentSnapshot,
    pub(super) workspace: WorkspaceSnapshot,
    pub(super) adapters: Vec<AdapterSnapshot>,
    pub(super) tests: TestSnapshot,
    pub(super) context_map: ContextMap,
    pub(super) evidence: std::collections::BTreeMap<EvidenceId, Evidence>,
    pub(super) diagnostics: Vec<ContextDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct RepositoryIdentity {
    pub(super) commit: String,
    pub(super) branch: String,
    pub(super) dirty: bool,
    pub(super) changed_paths: Vec<RepositoryPath>,
    pub(super) worktree_fingerprint: String,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct EnvironmentSnapshot {
    pub(super) host: String,
    pub(super) target: String,
    pub(super) enabled_features: Vec<String>,
    pub(super) rustc_version: Fact<String>,
    pub(super) cargo_version: Fact<String>,
    pub(super) nextest_version: Fact<String>,
    pub(super) generated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct WorkspaceSnapshot {
    pub(super) version: Fact<String>,
    pub(super) edition: Fact<String>,
    pub(super) msrv: Fact<String>,
    pub(super) packages: Vec<PackageSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct PackageSnapshot {
    pub(super) package_id: String,
    pub(super) name: String,
    pub(super) manifest_path: RepositoryPath,
    pub(super) targets: Vec<TargetSnapshot>,
    pub(super) features: Vec<String>,
    pub(super) dependencies: Vec<DependencySnapshot>,
    pub(super) metrics: SourceMetrics,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct DependencySnapshot {
    pub(super) package_name: String,
    pub(super) rename: Option<String>,
    pub(super) kind: DependencyKind,
    pub(super) target_predicate: Option<String>,
    pub(super) optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DependencyKind {
    Normal,
    Build,
    Dev,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct TargetSnapshot {
    pub(super) name: String,
    pub(super) kinds: Vec<String>,
    pub(super) source_path: RepositoryPath,
    pub(super) required_features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct SourceMetrics {
    pub(super) rust_files: usize,
    pub(super) physical_lines: usize,
    pub(super) non_empty_lines: usize,
    pub(super) included_roots: Vec<RepositoryPath>,
    pub(super) includes_generated: bool,
}
```

`RepositoryPath` serializes as a workspace-relative forward-slash path on every platform. Package
membership uses Cargo package IDs and manifest roots, never substring matching.

### Adapter Types

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AdapterMaturity {
    Production,
    Experimental,
    Blocked,
    Stub,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapabilitySupport {
    Yes,
    Limited,
    Blocked,
    No,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct AdapterSnapshot {
    pub(super) id: String,
    pub(super) maturity: Fact<AdapterMaturity>,
    pub(super) platforms: Fact<Vec<String>>,
    pub(super) default_roles: Fact<Vec<String>>,
    pub(super) registry_presence: Fact<bool>,
    pub(super) capabilities: std::collections::BTreeMap<String, Fact<CapabilitySupport>>,
}
```

The runtime registry observation validates identity, platform gating, and default/fallback names.
Capability observations come only from current profile test evidence. Source presence alone cannot
upgrade a capability to `yes`.

### Test Types

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TestDeclarationKind {
    Function,
    MacroInvocation,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct SourceTestDeclaration {
    pub(super) stable_id: String,
    pub(super) package_id: String,
    pub(super) path: RepositoryPath,
    pub(super) module_path: Vec<String>,
    pub(super) name: String,
    pub(super) cfg_predicates: Vec<String>,
    pub(super) kind: TestDeclarationKind,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct ExecutableTest {
    pub(super) stable_id: String,
    pub(super) package_id: String,
    pub(super) binary_id: String,
    pub(super) test_name: String,
    pub(super) ignored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProfileStatus {
    Validated,
    Failed,
    Unavailable,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct TestProfileResult {
    pub(super) profile_id: String,
    pub(super) target: String,
    pub(super) features: Vec<String>,
    pub(super) status: ProfileStatus,
    pub(super) executable_tests: Vec<ExecutableTest>,
    pub(super) unavailable_reason: Option<String>,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct TestSnapshot {
    pub(super) source_declarations: Vec<SourceTestDeclaration>,
    pub(super) profiles: Vec<TestProfileResult>,
    pub(super) validated_unique_tests: Vec<ExecutableTest>,
}
```

There is deliberately no bare `tests.total`. Consumers may count source declarations, tests in a
specific validated profile, or unique validated tests across current profiles. Macro invocations
are declarations, not guessed executable-test counts.

### Derived Context Map Types

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct ContextMap {
    pub(super) crates: Vec<CrateOwnership>,
    pub(super) files: Vec<FileOwnership>,
    pub(super) changed_files: Vec<FileOwnership>,
    pub(super) collector_tasks: Vec<CollectorTask>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct CrateOwnership {
    pub(super) package_id: String,
    pub(super) manifest_path: RepositoryPath,
    pub(super) source_roots: Vec<RepositoryPath>,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct FileOwnership {
    pub(super) path: RepositoryPath,
    pub(super) owner_package_id: Option<String>,
    pub(super) role: FileRole,
    pub(super) role_origin: RoleOrigin,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FileRole {
    CargoTarget,
    RustSource,
    Test,
    Example,
    Benchmark,
    Manifest,
    Documentation,
    Workflow,
    Configuration,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RoleOrigin {
    CargoMetadata,
    DeclaredRule,
    InferredPath,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct CollectorTask {
    pub(super) id: String,
    pub(super) depends_on: Vec<String>,
    pub(super) status: EvidenceStatus,
    pub(super) evidence_ids: Vec<EvidenceId>,
}
```

The complete tracked-file map is deterministic. `changed_files` is a focused projection from the
same records, not a second independently inferred mapping.

### Manifest Types And Functions

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub(super) struct ContextManifest {
    pub(super) schema_version: u32,
    pub(super) adapters: Vec<AdapterDeclaration>,
    pub(super) profiles: Vec<ValidationProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub(super) struct AdapterDeclaration {
    pub(super) id: String,
    pub(super) maturity: AdapterMaturity,
    pub(super) platforms: Vec<String>,
    pub(super) default_roles: Vec<String>,
    pub(super) capabilities: std::collections::BTreeMap<String, CapabilitySupport>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub(super) struct ValidationProfile {
    pub(super) id: String,
    pub(super) target: String,
    pub(super) features: Vec<String>,
    pub(super) no_default_features: bool,
    pub(super) all_targets: bool,
    pub(super) required_in_ci: bool,
}

pub(super) fn load_manifest(root: &std::path::Path) -> anyhow::Result<ContextManifest>;
pub(super) fn validate_manifest(manifest: &ContextManifest) -> anyhow::Result<()>;
pub(super) fn sync_adapter_matrix(
    root: &std::path::Path,
    manifest: &ContextManifest,
) -> anyhow::Result<()>;
```

Manifest validation rejects duplicate IDs, unknown maturity/support values, missing native
profiles, duplicate target/feature profiles, unknown default adapter IDs, and capability rows that
do not have the same key set across adapters.

### Collection And Output Functions

```rust
pub(super) fn collect_workspace(
    runner: &impl CommandRunner,
    repository: &impl RepositoryReader,
    root: &std::path::Path,
) -> anyhow::Result<WorkspaceSnapshot>;

pub(super) fn collect_source_tests(
    repository: &impl RepositoryReader,
    workspace: &WorkspaceSnapshot,
) -> anyhow::Result<Vec<SourceTestDeclaration>>;

pub(super) fn collect_test_profile(
    runner: &impl CommandRunner,
    root: &std::path::Path,
    profile: &ValidationProfile,
) -> anyhow::Result<TestProfileResult>;

pub(super) fn derive_context_map(
    identity: &RepositoryIdentity,
    workspace: &WorkspaceSnapshot,
    tracked_files: &[RepositoryPath],
    collector_tasks: Vec<CollectorTask>,
) -> ContextMap;

pub(super) fn validate_snapshot_json(value: &serde_json::Value) -> anyhow::Result<()>;

pub(super) fn persist_snapshot(
    root: &std::path::Path,
    snapshot: &ContextSnapshot,
    profile_evidence: &[TestProfileResult],
) -> anyhow::Result<std::path::PathBuf>;
```

## Data Flow

1. **Options**: `main.rs` parses context flags into `ContextOptions` without accepting positional
   values or a v2 schema selector.
2. **Declarations**: `manifest.rs`, Cargo metadata, and Rust AST parsing produce declared facts and
   evidence records.
3. **Native observations**: Git, tool-version commands, and nextest JSON produce observations for
   the current host profile.
4. **Additional observations**: `--validate-all` attempts configured profiles; current cache and
   `--evidence-dir` artifacts contribute only when their full cache keys match.
5. **Reconciliation**: collectors compare declarations and observations into `Fact<T>` values and
   append stable diagnostics for mismatches, failures, stale evidence, and unavailable profiles.
6. **Derivation**: Cargo package roots plus tracked files produce the ownership map; the executed
   collector DAG produces task slices.
7. **Validation**: the fully serialized value is validated against the embedded v3 JSON Schema
   before any output or persistence.
8. **Sink**: stdout receives one JSON document; `--save` writes the latest snapshot, history, and
   current profile evidence beneath ignored `artifacts/context/`.
9. **Exit**: default mode exits successfully with diagnostics; `--strict` returns nonzero after
   emitting JSON when required native evidence has errors, while `--validate-all --strict`
   requires current evidence for every configured profile.

## Evidence Freshness And Merge Rules

A profile-evidence cache key contains:

- Git commit;
- dirty-worktree fingerprint;
- `xtask/context.toml` content hash;
- target triple;
- sorted feature set and default-feature mode;
- Cargo, rustc, and nextest versions.

Evidence with any differing key component is `stale`. Stale records may be displayed for
diagnosis but cannot satisfy strict mode, validate adapter capability observations, or contribute
to `validated_unique_tests`.

Default mode refreshes repository facts, static declarations, and the native profile. Foreign
profiles are attempted only with `--validate-all`; same-key CI artifacts can satisfy a profile
that the local host cannot build. Without `--save`, collection remains read-only and does not
update the evidence cache.

## Hexagonal Boundaries

- **Process port**: `CommandRunner` isolates Git, Cargo, rustc, and nextest execution.
- **Repository port**: `RepositoryReader` isolates UTF-8 reads, tracked-file enumeration, and file
  metrics.
- **System adapters**: `SystemCommandRunner` and `SystemRepositoryReader` provide production I/O.
- **Pure transforms**: metadata parsing, syntax inventory, reconciliation, context derivation,
  cache-key calculation, and schema-model construction consume captured inputs and require no I/O.
- **Output adapter**: `output.rs` owns JSON Schema validation, artifact persistence, and generated
  documentation replacement.

The external parser libraries (`cargo_metadata`, `syn`, and `jsonschema`) are deterministic
in-process transforms rather than I/O dependencies. They do not require additional ports.

## Documentation Synchronization

`cargo xtask docs sync-adapters` replaces only a marked generated block in
`docs/core/FEATURE_MATRIX.mbx.md`. `cargo xtask docs audit` fails when the block differs from the
canonical manifest, so hand edits cannot silently reintroduce drift. Narrative notes and source
references outside the generated block remain hand-maintained.

`docs/core/XTASK_CLI.mbx.md` documents the v3 field semantics and explicitly warns that:

- maturity/capability declarations are policy metadata;
- source declarations are not executable tests;
- unavailable profiles are not zero-test profiles;
- dirty snapshots describe a worktree fingerprint, not only HEAD;
- `--strict` and `--validate-all --strict` have different completeness requirements.

## Test Strategy

- **Unit**: parse exact Cargo package IDs, dependency kinds/renames/target predicates, nextest JSON,
  Rust test attributes/macros, manifest validation, normalized paths, evidence keys, and strict
  severity thresholds.
- **Regression**: pin duplicate dependency elimination within a kind, legitimate self dev-
  dependencies, MSRV/runtime separation, adapter maturity mismatches, and removal of bare test
  totals.
- **Failure injection**: command not found, nonzero process status, invalid UTF-8, malformed JSON,
  missing target, stale CI artifact, dirty worktree, and schema rejection all yield deterministic
  diagnostics rather than empty collections.
- **Integration**: run the default and strict command against a fixture workspace; run
  `--validate-all` with fake available/unavailable profiles; verify `--save` and evidence import.
- **Golden schema**: serialize a complete v3 fixture and validate it against
  `xtask/schema/cli.schema.json`; mutate each required section to prove rejection.
- **Repository contract**: validate the live manifest against adapter registry IDs and the
  generated feature-matrix block.
- **Determinism**: identical captured inputs produce byte-identical JSON except for the explicitly
  injected collection timestamp.

No property, fuzz, or model-check target is required initially: parsing libraries own low-level
syntax correctness, while fixture and mutation tests cover this command's bounded transformations.

## Compatibility And Migration

- Snapshot v3 replaces v2 outright; no `--schema-version 2` mode is provided.
- `cargo xtask info context` and the deprecated `cargo xtask context` alias remain valid.
- Existing `artifacts/context/snapshot.json` is overwritten only with schema-valid v3 output.
- Existing v2 `history.jsonl` records remain historical data; each line is self-identifying by
  `snapshot_version`.
- Machine consumers must migrate from `workspace.rust_version`, `crates[].deps`,
  `crates[].test_count`, `tests.total`, and fixed context-map assignments to the v3 sections.

## Out Of Scope

- Changing adapter implementations, runtime selection, daemon wiring, or capability behavior.
- Refactoring the duplicate adapter list in `crates/mbx/src/commands/doctor.rs`.
- Proving runtime quality from source presence or documentation claims.
- Enumerating the complete Cargo feature power set.
- Claiming foreign-target executable counts from static source analysis.
- Preserving v2 output compatibility.
- Updating unrelated repository documentation, workflow edits, or pre-existing dirty files.
- Replacing `cargo xtask info metrics`; its annotation metric remains a separately defined tool.

## Risk

- [x] **Breaking serialized API**: yes -- snapshot v3 intentionally removes ambiguous v2 fields.
- [ ] **Breaking Rust library API**: no -- all new APIs are internal to the `xtask` binary.
- [x] **New external dependencies**: yes -- `cargo_metadata` for typed metadata, `syn` for source
      declarations, and `jsonschema` for validating every emitted snapshot.
- [ ] **New workspace crate**: no.
- [ ] **Product feature flag**: no.
- [x] **Cross-platform risk**: profile evidence depends on target/linker availability and therefore
      uses explicit `unavailable` and CI merge states.
- [x] **Performance risk**: full profile validation builds test binaries; it remains opt-in through
      `--validate-all` and content-keyed evidence reuse.
- [x] **Documentation risk**: generated adapter rows replace one hand-maintained section, guarded
      by `docs audit` and marked-block replacement.
- [x] **Secret risk**: external command output may contain sensitive environment details; persisted
      evidence stores only safe locators, hashes, statuses, and redacted diagnostics.

## Implementation Constraints

- Before any commit, run `git branch --show-current` and stop if it returns `main`.
- Never use `--no-verify`; never alter Git signing configuration.
- Run `cargo fmt --all`, re-stage, `cargo clippy --workspace -- -D warnings`, and
  `cargo nextest run --workspace` before committing.
- Stop after three failed attempts at the same test or fix and report the root cause.
- Store temporary investigation artifacts only in `.ctx/_WORKING_DIR/`.
- Preserve all unrelated working-tree changes present when implementation begins.
