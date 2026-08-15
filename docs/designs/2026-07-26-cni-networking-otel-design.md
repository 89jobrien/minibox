# Design: minibox-cni — CNI-spec bridge networking + OTEL span instrumentation

## Goal

Give the native Linux adapter real CNI-spec-compliant bridge networking (replacing the bespoke
`BridgeNetwork`/`IpAllocator`/DNAT implementation) via a new `minibox-cni` crate implementing
`minibox-core`'s existing `NetworkProvider` port, and add `#[instrument]` OTEL spans across the
daemon request boundary, adapter trait calls, and the new CNI plugin chain so the already-built
(but currently unused) OTLP exporter in `crates/minibox/src/daemon/telemetry/traces.rs` has
something meaningful to export.

## Approved Approach

"Full CNI Compliance, isolated crate, OS-agnostic protocol layer" from brainstorm: hand-roll the
CNI exec protocol and full chain orchestration in a new leaf crate; the crate is deliberately
ignorant of how a network namespace is obtained (`target: &str` passed through opaquely), keeping
it portable to a future non-Linux (WinCNI/HNS) adapter without modification. Combined with OTEL
instrumentation across the same call chain, landing structured error fields (not just a summary
string) on plugin-failure spans.

## Crate Ownership

- **Owner crate**: `minibox-cni` (new, `crates/minibox-cni`, `publish = false`) — CNI exec
  protocol and chain orchestration is a self-contained concern with no production consumers
  outside adapter wiring; keeping it a leaf crate matches the `minibox-bench` precedent and keeps
  `minibox-core` free of process-spawning/plugin-protocol code.
- **Affected crates**:
  - `minibox-core` — no changes; `minibox-cni` consumes its existing `NetworkProvider`,
    `NetworkConfig`, `NetworkMode`, `PortMapping`, `NetworkStats` types unchanged.
  - `minibox` — `crates/minibox/src/adapters/network/bridge.rs`'s `BridgeNetwork` stops being
    constructed for the native adapter (see Integration Points); `crates/minibox/src/daemon/
    handler/run.rs` and `crates/minibox/src/adapters/runtime.rs` gain `#[instrument]`.
  - `miniboxd` — one `HandlerDependencies` construction site (native suite) swaps its
    `NetworkProvider` implementation.
  - `mbx` — `crates/mbx/src/commands/doctor.rs` gains a CNI plugin/`CNI_PATH` preflight check.
- No crate imports `minibox-cni` except `minibox` (and `miniboxd` transitively via `minibox`).
  Dependency direction: `minibox-core` ← `minibox-cni` ← `minibox`. Acyclic (`minibox` already
  depends on `minibox-core` directly too, same shape as `minibox-macros`).

## Public API

### Types (`minibox-cni::config`)

```rust
/// Parsed CNI network configuration list (.conflist format).
pub struct NetworkConfigList {
    pub cni_version: String,
    pub name: String,
    pub plugins: Vec<PluginConfig>,
}

/// A single plugin's config within a .conflist chain.
pub struct PluginConfig {
    pub plugin_type: String,
    pub raw: serde_json::Value,
}
```

### Types (`minibox-cni::result`)

```rust
/// Merged result of a CNI ADD chain (final prevResult).
pub struct CniResult {
    pub cni_version: String,
    pub interfaces: Vec<CniInterface>,
    pub ips: Vec<CniIpConfig>,
    pub dns: CniDns,
}

pub struct CniInterface {
    pub name: String,
    pub mac: Option<String>,
    pub sandbox: Option<String>,
}

pub struct CniIpConfig {
    pub address: String,
    pub gateway: Option<String>,
    pub interface: Option<usize>,
}

pub struct CniDns {
    pub nameservers: Vec<String>,
    pub domain: Option<String>,
    pub search: Vec<String>,
    pub options: Vec<String>,
}
```

### Types (`minibox-cni::error`)

```rust
pub enum CniError {
    PluginNotFound { plugin: String, searched: Vec<PathBuf> },
    PluginError { plugin: String, code: Option<u32>, msg: String, details: Option<String> },
    ProcessFailed { plugin: String, exit_code: Option<i32>, stderr: String },
    ConfigParse(serde_json::Error),
    Io(std::io::Error),
}
```

`CniError` derives `thiserror::Error` + `miette::Diagnostic` per repo convention (`rust-patterns.md`).
It does not depend on `anyhow` — conversion to whatever `NetworkProvider`'s method `Result` alias
uses happens at the `CniNetworkProvider` impl boundary via `?`/`.into()`.

### Types (`minibox-cni::provider`)

```rust
/// Adapter implementing minibox-core's NetworkProvider port via CNI plugin chains.
pub struct CniNetworkProvider {
    pub cni_path: Vec<PathBuf>,
    pub config_dir: PathBuf,
}
```

### Functions

```rust
impl NetworkConfigList {
    pub fn from_file(path: &Path) -> Result<Self, CniError>;

    /// Runs the full ADD chain in plugin order, threading prevResult between plugins.
    /// On mid-chain failure, automatically rolls back (DEL in reverse) already-succeeded
    /// plugins before returning the error.
    pub async fn add(
        &self,
        cni_path: &[PathBuf],
        netns: &str,
        container_id: &str,
        ifname: &str,
    ) -> Result<CniResult, CniError>;

    /// Runs the DEL chain in reverse plugin order. Individual plugin DEL failures are
    /// logged (tracing::warn!) and do not short-circuit remaining teardown steps.
    pub async fn del(
        &self,
        cni_path: &[PathBuf],
        netns: &str,
        container_id: &str,
        ifname: &str,
    ) -> Result<(), CniError>;
}

impl CniNetworkProvider {
    pub fn new(cni_path: Vec<PathBuf>, config_dir: PathBuf) -> Self;
}

#[async_trait]
impl NetworkProvider for CniNetworkProvider {
    async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String>;
    async fn attach(&self, container_id: &str, pid: u32) -> Result<()>;
    async fn cleanup(&self, container_id: &str) -> Result<()>;
    async fn stats(&self, container_id: &str) -> Result<NetworkStats>;
}
```

`Result<T>` in the `NetworkProvider` impl matches whatever alias `minibox-core::domain::networking::
NetworkProvider` already declares (confirmed via context-map to be the existing trait's own
`Result` — not re-specified here; no change to the trait itself).

No new trait beyond re-implementing the existing `NetworkProvider` port. Considered adding an
internal `PluginExecutor` trait to allow mocking process-spawn in tests, but decided against it
(YAGNI): crate-level tests exercise the real exec path against fixture plugin binaries (small
shell scripts on a temp `CNI_PATH` emitting canned JSON), which tests the actual protocol
end-to-end without needing a real netns/root and without extra abstraction.

## Data Flow

1. **Source**: `mbx run` → `DaemonRequest::Run` → `handle_run` (`crates/minibox/src/daemon/
   handler/run.rs:129`).
2. **Transform**: `handle_run` calls `ContainerRuntime::spawn_process` (`LinuxNamespaceRuntime`,
   `crates/minibox/src/adapters/runtime.rs:109`), which creates namespaces (including
   `CLONE_NEWNET` via `NamespaceConfig::to_clone_flags()`) and returns a `SpawnResult` with a PID.
3. **Transform**: after PID is known, the native suite's configured `NetworkProvider` (now
   `CniNetworkProvider` instead of `BridgeNetwork`) is called: `setup(container_id, config)` then
   `attach(container_id, pid)`.
4. **Transform**: `CniNetworkProvider::attach` resolves `/proc/{pid}/ns/net`, loads the
   `.conflist` from `config_dir`, and calls `NetworkConfigList::add(cni_path, netns_path,
   container_id, ifname)`.
5. **Transform**: `NetworkConfigList::add` walks `plugins` in order (`bridge` → `host-local` →
   `portmap` → `dnsname`), spawning each via `tokio::process::Command`, threading `prevResult`
   between them, rolling back on mid-chain failure.
6. **Sink**: the merged `CniResult` (IPs, DNS) is recorded against the container; on stop/rm,
   `cleanup()` → `NetworkConfigList::del()` reverses the chain (best-effort, logged not
   propagated).

## Hexagonal Boundaries

- **Port** (trait, pre-existing, unchanged): `NetworkProvider` in
  `minibox-core::domain::networking` (`crates/minibox-core/src/domain/networking.rs:83-93`).
- **Adapter** (impl, being removed from the native suite's construction, code itself out of scope
  for deletion in this design — see Out of Scope): `BridgeNetwork` in
  `crates/minibox/src/adapters/network/bridge.rs:81`.
- **Adapter** (impl, new): `CniNetworkProvider` in `minibox-cni::provider`.

## Integration Points

- **`Cargo.toml`** (root): add `"crates/minibox-cni"` to `[workspace.members]`; add
  `minibox-cni = { path = "crates/minibox-cni" }` to `[workspace.dependencies]`.
- **`crates/minibox-cni/Cargo.toml`** (new): `version.workspace = true` / `edition.workspace =
  true` / `license.workspace = true` / `rust-version.workspace = true` /
  `repository.workspace = true`, `publish = false`, `[lints] workspace = true`. Dependencies:
  `minibox-core`, `tokio`, `serde`, `serde_json`, `async-trait`, `thiserror`, `miette`,
  `tracing` — all already workspace deps, no new external Cargo dependency introduced.
  Dev-dependency: `tempfile` (fixture `CNI_PATH` dirs in tests).
- **`crates/minibox/Cargo.toml`**: add `minibox-cni = { workspace = true }`.
- **`crates/miniboxd/src/main.rs`**: exactly one of the confirmed `HandlerDependencies`
  construction sites (lines 779 / 847 / 962 / 1019 — the native-suite one; exact line to be
  pinned down at planning time by matching adapter context) changes its `NetworkProvider`
  construction from `BridgeNetwork::new(...)` to `CniNetworkProvider::new(cni_path, config_dir)`.
  The other three (gke/colima/smolvm-or-krun) are untouched — this design is native-adapter-only
  per brainstorm scope.
- **`crates/minibox/src/daemon/handler/run.rs:129`**: `handle_run` gains
  `#[instrument(skip(state, deps), fields(container_id = %..., image = %params.image))]`,
  matching the existing pattern already used in `crates/minibox/src/daemon/handler/image.rs:77,146`
  (the only current `#[instrument]` usage in the codebase — confirmed by context-map, so this is
  imitating an established pattern, not inventing one).
- **`crates/minibox/src/adapters/runtime.rs:109`**: `LinuxNamespaceRuntime::spawn_process` gains
  `#[instrument(err, skip(config), fields(container_id = %config.container_id))]` (exact skipped/
  recorded fields to be finalized at planning time against the real `ContainerSpawnConfig` shape).
- **`minibox-cni` internals**: `NetworkConfigList::add`/`del` and the internal per-plugin exec
  function both get `#[instrument(err, fields(plugin = %name, exit_code = tracing::field::Empty,
  cni_error_code = tracing::field::Empty, cni_error_msg = tracing::field::Empty, stderr =
  tracing::field::Empty))]`, with `Span::current().record(...)` called immediately before
  returning `Err` to populate the empty fields — additive to `#[instrument(err)]`'s automatic
  Display-string capture, not a replacement.
- **`crates/mbx/src/commands/doctor.rs`**: `execute()` gains a new check (alongside the existing
  `compiled_adapters()`/`selected_adapter()` checks) reporting whether `CNI_PATH` is set and
  whether the required plugin binaries (`bridge`, `host-local`, `portmap`, `dnsname`) are present
  on it — advisory only, matching the existing checks' style (no live binary invocation, just
  presence).

## Out of Scope

- GKE, smolvm, colima, krun adapters — none expose a host-visible per-container network
  namespace (confirmed: GKE reports `supports_network_isolation: false`; smolvm/colima delegate
  networking entirely to opaque VM-level tooling). Their networking gap is a separate future
  problem, not addressed here.
- Deleting `crates/minibox/src/adapters/network/bridge.rs` (`IpAllocator`/`BridgeNetwork`/
  `apply_port_mappings`) — this design stops it from being *constructed* for the native suite,
  but leaves physical removal of the module (and its tests) as a follow-up cleanup once
  `CniNetworkProvider` is validated in practice. No dual-path *fallback* is introduced (the native
  suite only ever constructs one `NetworkProvider`), but the old code isn't deleted in this pass.
- Windows/HNS-based CNI adapter implementation — only the seam (`target: &str`/netns-path opacity
  inside `minibox-cni`) that would allow one later without modifying this crate.
- OTEL sampling strategy — `OtelGuard` stays always-on/unsampled; a sampler can be added later
  behind the existing `otel` feature flag if span volume becomes a problem.
- Rollout feature-gating for `CniNetworkProvider` itself (see Risk below — flagged as an open
  question for approval, not decided in this design).

## Risk

- [ ] **Breaking API changes**: No Rust `pub` signature breaks. However, this is a **runtime
      behavioral break**: operators using the native adapter's bridge networking today must have
      CNI plugin binaries (`bridge`, `host-local`, `portmap`, `dnsname` — the standard
      `containernetworking/plugins` release) installed and `CNI_PATH` set, or bridge networking
      stops working entirely (fail-fast per brainstorm's Section 3, no silent fallback to the old
      implementation). This should be called out prominently in `docs/PLATFORM_SUPPORT.md` and
      release notes.
- [ ] **New external dependency**: No new Cargo/crates.io dependency (all of `minibox-cni`'s deps
      are already workspace deps). **Yes** for a new *runtime system* dependency: the four CNI
      plugin binaries become a hard prerequisite for native-adapter bridge networking.
- [x] **Feature flag required**: Yes — decided. A new `cni` Cargo feature on `minibox` (and
      forwarded through `miniboxd`, matching the existing `otel`/`metrics` optional-feature
      pattern) gates `CniNetworkProvider` construction at the native suite's `HandlerDependencies`
      site. Default **off** until plugin-binary distribution/docs land. This is a transitional
      rollout safety valve only — distinct from "keeping the old implementation as a maintained
      parallel path" (explicitly rejected in brainstorming). With the feature off, the native
      suite keeps constructing `BridgeNetwork` exactly as today; with it on, it constructs
      `CniNetworkProvider` instead. No runtime toggle, no dual construction at once.

## Evidence / Context-Map Findings

- `NetworkProvider` port: `crates/minibox-core/src/domain/networking.rs:83-93`
- `NetworkMode`/`PortMapping`/`NetworkConfig`: `crates/minibox-core/src/domain/networking.rs:13-25,
  134-172, 192-205` (canonical; `crates/minibox/src/domain/networking.rs` has a legacy duplicate
  missing `tailnet_*` fields — not touched by this design)
- Current adapter to be replaced: `BridgeNetwork` impl of `NetworkProvider` at
  `crates/minibox/src/adapters/network/bridge.rs:251`; `TODO(#229)` at line 2
- Netns creation: `crates/minibox/src/container/namespace.rs:1-80` (`NamespaceConfig`,
  `to_clone_flags()`); netns join precedent: `crates/minibox/src/adapters/exec.rs:3,397`
  (`setns(2)` against `/proc/{pid}/ns/*`)
- `ContainerRuntime` port: `crates/minibox-core/src/domain/runtime.rs:189`; native impl
  `LinuxNamespaceRuntime` at `crates/minibox/src/adapters/runtime.rs:84,98,109`
- `handle_run`: `crates/minibox/src/daemon/handler/run.rs:129`; existing `#[instrument]` precedent:
  `crates/minibox/src/daemon/handler/image.rs:77,146`
- OTEL plumbing: `crates/minibox/src/daemon/telemetry/traces.rs` (`init_tracing:25`,
  `OtelGuard:101-105`); invoked from `crates/miniboxd/src/main.rs:351-356,516,518`
- Workspace members: root `Cargo.toml:3-18`; `HandlerDependencies` construction sites in
  `crates/miniboxd/src/main.rs` at lines 779, 847, 962, 1019 (4 confirmed literal constructions)
- `mbx doctor`: `crates/mbx/src/commands/doctor.rs` — `execute()`, `compiled_adapters()`,
  `selected_adapter()`, `adapter_entries()`, `run_xtask_doctor()`
- Dependency direction confirmed acyclic: `crates/minibox/Cargo.toml:10-37` (depends on
  `minibox-core`), `crates/minibox-core/Cargo.toml:27-65` (no dependency back on `minibox`)
- Reference Cargo.toml shapes: `crates/minibox-bench/Cargo.toml` (`publish = false`, no
  `[lints]`), `crates/mcp/Cargo.toml` (`publish = true`, has `[lints] workspace = true`) —
  `minibox-cni` follows the `publish = false` + `[lints]` combination (leaf internal crate, but
  keep lint conformance)
