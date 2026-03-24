# Networking Cleanup Design

**Date:** 2026-03-24
**Status:** Approved

## Overview

Four improvements identified from devloop analysis of the container networking feature:

1. Extract `NetworkLifecycle` module from `handler.rs`
2. Document `NetworkMode::None` semantics across domain, protocol, and CLI
3. Add `NetworkProvider` conformance tests
4. Add Colima env var + manifest.json regression tests

The `minibox-lib` → `linuxbox` rename audit was completed and found no issues.

---

## 1. NetworkLifecycle Extraction

### Problem

`handler.rs` (~950 lines) has networking calls scattered across three execution paths at five call sites:

- `run_inner`: `setup` (line ~542), `attach` (line ~614) — inside a `tokio::task::spawn` closure
- `run_inner_capture`: `setup` (line ~349), `attach` (line ~409)
- `handle_run_streaming`: `cleanup` (line ~228)
- `handle_stop`: `cleanup` (line ~772)
- `remove_inner`: `cleanup` (line ~909)

Error handling and cleanup logic is duplicated. `HandlerDependencies` is at risk of becoming a god object.

### Solution

Create `crates/daemonbox/src/network_lifecycle.rs` — a thin wrapper that consolidates setup/attach/cleanup into one place with consistent error handling.

```rust
#[derive(Clone)]
pub struct NetworkLifecycle {
    provider: DynNetworkProvider,
}

impl NetworkLifecycle {
    pub fn new(provider: DynNetworkProvider) -> Self { ... }

    /// Set up network namespace for a new container. Returns the namespace path.
    pub async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> { ... }

    /// Attach a running container process to its network namespace.
    pub async fn attach(&self, container_id: &str, pid: u32) -> Result<()> { ... }

    /// Tear down networking for a container. Best-effort: logs warn on error, never propagates.
    pub async fn cleanup(&self, container_id: &str) { ... }
}
```

`NetworkLifecycle` derives `Clone` because `run_inner` constructs it before a `tokio::task::spawn` closure and the struct must be `Send + 'static`. `DynNetworkProvider` is `Arc<dyn NetworkProvider + Send + Sync>`, which is `Clone`, so the derive is sound.

`HandlerDependencies.network_provider: DynNetworkProvider` is unchanged (no public API break). All five call sites in handler.rs are replaced with `NetworkLifecycle` method calls:

- `run_inner` and `run_inner_capture` construct `NetworkLifecycle::new(deps.network_provider.clone())` at entry and call `setup` / `attach`
- `handle_run_streaming`, `handle_stop`, and `remove_inner` construct `NetworkLifecycle::new(deps.network_provider.clone())` and call `cleanup`

### Scope

- New file: `crates/daemonbox/src/network_lifecycle.rs`
- Modify: `crates/daemonbox/src/handler.rs` (replace all 5 inline network call sites)
- Modify: `crates/daemonbox/src/lib.rs` (declare module)
- No changes to `HandlerDependencies` public API
- No changes to any other crate

---

## 2. NetworkMode / NetworkConfig Documentation

### Problem

`NetworkMode::None` is the default but its semantics are underspecified. Brief doc comments exist on the enum variants but don't explain the isolation model or default wiring. Callers reading `DaemonRequest::Run.network: Option<NetworkConfig>` cannot tell whether `None` means "no network," "host network," or "undefined behavior."

### Solution

Expand existing `///` doc comments:

**`linuxbox/src/domain/networking.rs`** — `NetworkMode` enum variants:
- `None` — Container gets an isolated network namespace with no interfaces configured. Cannot reach the host network or internet. **This is the default.**
- `Bridge` — veth pair attached to a Linux bridge (`minibox0` by default). Container gets a private IP and can reach the host via the bridge.
- `Host` — Container shares the host network namespace. All host ports are accessible.
- `Tailnet` — Container connected via Tailscale/tsnet.

**`NetworkConfig`** — document that `Default` yields `NetworkMode::None` with sensible bridge/subnet defaults for when a real network mode is later selected.

**`linuxbox/src/protocol.rs`** — `DaemonRequest::Run.network` field:
- Document that `None` (`Option::None`) maps to `NetworkConfig::default()`, which selects `NetworkMode::None` (isolated namespace, no connectivity).

**`minibox-cli/src/main.rs`** — expand `--network` flag help text (brief version already exists):
- Append: "'none' runs the container in an isolated namespace with no network connectivity."

### Scope

- Modify: `crates/linuxbox/src/domain/networking.rs`
- Modify: `crates/linuxbox/src/protocol.rs`
- Modify: `crates/minibox-cli/src/main.rs`
- Doc comment expansions only, no logic changes

---

## 3. NetworkProvider Conformance Tests

### Problem

`conformance_tests.rs` validates all domain adapter traits (ImageRegistry, FilesystemProvider, ResourceLimiter, ContainerRuntime, Handler) but has no *direct trait-level* tests for `NetworkProvider`. While handler conformance tests exercise the provider indirectly via `MockNetwork`, there are no tests that call `setup`/`attach`/`cleanup` directly on the provider, documenting the contract at the conformance level.

`handler_tests.rs` already has `test_network_setup_called_on_run` verifying setup call-count via `MockNetwork`. The conformance suite should include a matching assertion at the conformance level — it documents the requirement independently of the handler test suite.

### Solution

Add a `NetworkProvider` conformance section to `crates/daemonbox/tests/conformance_tests.rs` using `MockNetwork` (already exists in `linuxbox::adapters::mocks`):

```rust
// NetworkProvider conformance — direct trait-level tests
async fn network_noop_must_succeed_for_none_mode()
async fn network_setup_must_return_namespace_path()
async fn network_cleanup_must_succeed_after_setup()

// Handler conformance — verifies provider is wired into run path
// (mirrors test_network_setup_called_on_run in handler_tests.rs at conformance level)
async fn handler_run_must_invoke_network_setup()

// Performance conformance
async fn network_noop_setup_must_complete_under_1ms()
```

### Scope

- Modify: `crates/daemonbox/tests/conformance_tests.rs`
- No new files

---

## 4. Colima Env Var + Manifest.json Regression Tests

### Problem

The `colima_home()` / `lima_home()` / `limactl_command()` helpers added in the latest commit are private functions. They are not tested. The manifest.json-based layer extraction path in `get_image_layers` replaced fragile digest-guessing and also lacks dedicated regression coverage.

### Solution

**Env var resolution tests** — add to `crates/linuxbox/src/adapters/colima.rs` `#[cfg(test)] mod tests` (the only location that can access private functions):

```rust
fn colima_home_defaults_to_home_dot_colima()
fn colima_home_respects_colima_home_env_var()
fn lima_home_defaults_to_colima_home_lima()
fn lima_home_respects_lima_home_env_var()
fn limactl_command_injects_lima_home_into_env()
```

These join the existing `static ENV_MUTEX: Mutex<()>` guard already present in `colima.rs`'s inline `mod tests` (not in `adapter_colima_tests.rs`). All env var mutations are serialized through this mutex.

**Manifest.json extraction tests** — add to `crates/linuxbox/tests/adapter_colima_tests.rs` (can test `ColimaRegistry` via injected executor):

```rust
fn get_image_layers_parses_manifest_json_to_locate_layers()
fn get_image_layers_returns_error_on_malformed_manifest()
fn get_image_layers_returns_error_on_empty_layers_array()
```

These inject a mock executor returning canned `manifest.json` and simulate the layer extraction call sequence, verifying the manifest-based path rather than the old short-digest guessing.

### Scope

- Modify: `crates/linuxbox/src/adapters/colima.rs` (env var tests in inline `mod tests`)
- Modify: `crates/linuxbox/tests/adapter_colima_tests.rs` (manifest.json extraction tests)
- No new files

---

## Execution Order

1. **`network_lifecycle.rs`** — structural extraction; reduces noise in handler.rs before tests are written
2. **Doc comments** — independent, no ordering constraint
3. **Conformance tests** — independent of extraction (test observable behavior, not internal structure); can proceed in parallel with 1–2
4. **Colima tests** — fully independent

## Non-Goals

- No new network adapters (Bridge, Host, Tailnet implementations)
- No changes to `HandlerDependencies` public API
- Cleanup calls in `handle_stop` and `remove_inner` are replaced with `NetworkLifecycle.cleanup` (in scope), but no other changes to those functions
- No CI pipeline changes
