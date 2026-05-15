# Code Style and Conventions

Minibox is a Rust 2024 container runtime. This document records the patterns observed throughout
the codebase — follow them when adding or modifying code.

---

## Table of Contents

1. [Workspace Layout](#1-workspace-layout)
2. [Module Documentation](#2-module-documentation)
3. [Error Handling](#3-error-handling)
4. [Tracing](#4-tracing)
5. [Architecture — Hexagonal Ports and Adapters](#5-architecture--hexagonal-ports-and-adapters)
6. [Platform Gating](#6-platform-gating)
7. [Unsafe Code](#7-unsafe-code)
8. [Async / Sync Boundary](#8-async--sync-boundary)
9. [Security Invariants](#9-security-invariants)
10. [Naming Conventions](#10-naming-conventions)
11. [Imports and Visibility](#11-imports-and-visibility)
12. [Section Dividers](#12-section-dividers)
13. [Serialization](#13-serialization)
14. [Testing](#14-testing)
15. [Macros](#15-macros)
16. [Typestate Pattern](#16-typestate-pattern)
17. [Protocol Changes](#17-protocol-changes)

---

## 1. Workspace Layout

```
crates/
  minibox-core/     # Cross-platform shared types, domain traits, protocol, image handling
  minibox/          # Infrastructure adapters + container runtime (Linux-native + cross-platform)
  miniboxd/         # Daemon binary: socket listener, handler dispatch, adapter wiring
  mbx/              # CLI binary: command implementations, terminal handling
  minibox-macros/   # macro_rules! boilerplate reduction (as_any!, adapt!, etc.)
  minibox-conformance/ # Conformance test harness and runner
  macbox/           # macOS adapter: krun/smolvm backends
  winbox/           # Windows adapter: HCS/WSL2 backends
  minibox-crux-plugin/ # crux runtime plugin
xtask/              # Cargo xtask: pre-commit, verify, prepush, cleanup gates
```

Key conventions:
- `minibox-core` has **zero infrastructure dependencies** — only `std`, `serde`, `tokio`, `anyhow`,
  `thiserror`, and `tracing`. Never add OS-specific imports here.
- `minibox` re-exports everything from `minibox-core` that adapters or macros need. Do not remove
  these re-exports; macro expansion depends on them.
- Platform-specific crates (`macbox`, `winbox`) implement domain traits from `minibox-core`.

---

## 2. Module Documentation

Every module file begins with a `//!` inner doc comment. A complete module doc includes:

- One-sentence purpose statement.
- A `# Architecture` or structural diagram for non-trivial modules.
- A `# Traits (Ports)` or `# Adapters` section listing public items.
- A code example (using `rust,ignore` when the example can't compile standalone).

**Example — well-formed module doc:**

```rust
//! Container lifecycle event types and pub/sub ports.
//!
//! `EventSink` is the write port — handlers call `emit()`.
//! `EventSource` is the read port — consumers (CLI, dashboards) subscribe.
//! `BroadcastEventBroker` is the single adapter implementing both ports.
```

**Example — well-formed lib.rs with module table:**

```rust
//! # minibox-core
//!
//! Cross-platform shared types, domain traits, protocol definitions, and
//! image handling for the Minibox container runtime.
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`domain`] | Hexagonal-architecture ports: trait definitions. Zero infrastructure deps. |
//! | [`protocol`] | Newline-delimited JSON types for the Unix socket protocol. |
```

Inline items (structs, functions, constants) use `///` doc comments. Required fields:

- Every `pub struct` field gets a `///` comment explaining its meaning, not just restating its type.
- Every `pub fn` on an `impl` block documents what it does, key preconditions, and on `Result`
  returns, what can fail.

---

## 3. Error Handling

### Fine-grained typed errors with `thiserror`

Each subsystem defines its own `#[derive(Debug, Error)]` enum with named-field variants for all
structured failure cases. Variants include enough context to diagnose the failure without a
stack trace.

```rust
// ✅ Correct
#[derive(Debug, Error)]
pub enum ImageError {
    #[error("image {name}:{tag} not found in local store")]
    NotFound { name: String, tag: String },

    #[error("failed to write to image store at {path}: {source}")]
    StoreWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
```

Rules:
- Use named fields (`{ path: String }`) not tuple variants for anything beyond a bare wrapper.
- Always include `#[source]` on the field that is a wrapped lower-level error.
- Include `Other(String)` as a catch-all last variant for un-enumerated cases.
- Cross-platform error types live in `minibox-core/src/error.rs`. Linux-specific errors
  (anything depending on `nix`) live in `minibox/src/error.rs`.

### `anyhow` for propagation

Production functions return `anyhow::Result<T>`. Always attach context to every `?`:

```rust
// ✅ Correct — static context
let content = fs::read_to_string(path).context("read image manifest")?;

// ✅ Correct — dynamic context with path or ID
let content = fs::read_to_string(path)
    .with_context(|| format!("read manifest for {}", path.display()))?;

// ❌ Wrong — no context
let content = fs::read_to_string(path)?;

// ❌ Wrong — .unwrap() in production code
let content = fs::read_to_string(path).unwrap();
```

Use `anyhow::bail!` for early returns that don't fit a typed error variant:

```rust
if self.state != ContainerState::Running {
    bail!("container {} is not running", self.id);
}
```

### Cleanup on failure

Resource-creating functions must clean up on error. Log secondary cleanup failures at `warn!`
and do not propagate them — let the original error surface.

```rust
let rootfs = create_overlay(&config.layers, &id).context("create_overlay")?;
if let Err(e) = setup_cgroup(&id, &config.limits) {
    if let Err(cleanup_err) = destroy_overlay(&id) {
        tracing::warn!(
            container_id = %id,
            error = %cleanup_err,
            "container: overlay cleanup failed after cgroup error"
        );
    }
    return Err(e).context("setup_cgroup");
}
```

---

## 4. Tracing

All daemon and library code uses `tracing` macros. **Never use `println!` or `eprintln!`** in
daemon code — they contaminate container stdio.

### Structured fields are mandatory

Values go in **key = value** fields, not in the message string:

```rust
// ✅ Correct
tracing::info!(
    container_id = %id,
    pid = pid.as_raw(),
    rootfs = %config.rootfs.display(),
    "container: process started"
);

// ❌ Wrong — values embedded in message (not queryable)
tracing::info!("Container {} started with PID {}", id, pid);
```

Use `%` for types that implement `Display`, `?` for `Debug`.

### Message format

Messages use lowercase `noun: verb` (or `subsystem: action`) format:

- `"container: process started"`
- `"tar: rejected absolute symlink"`
- `"adapter: MINIBOX_ADAPTER=native requires root"`
- `"image: pulled {image}:{tag}"`

No sentence case. No trailing period.

### Severity levels

| Level    | Use                                                                              |
| -------- | -------------------------------------------------------------------------------- |
| `error!` | Unrecoverable: crash, fatal exec error, daemon cannot continue                   |
| `warn!`  | Security rejections, degraded behaviour, best-effort cleanup failures            |
| `info!`  | Lifecycle milestones: container start/stop, image pull phases, overlay mount     |
| `debug!` | Syscall arguments, byte counts, internal state transitions                       |

---

## 5. Architecture — Hexagonal Ports and Adapters

Minibox follows the **hexagonal (ports and adapters)** pattern:

```
Domain Layer (minibox-core/src/domain.rs)
  → Defines trait "ports": ImageRegistry, FilesystemProvider, ResourceLimiter, ContainerRuntime
  → Zero infrastructure imports

Infrastructure Adapters (minibox/src/adapters/)
  → Implement domain traits for real systems: DockerHubRegistry, OverlayFilesystem, etc.
  → Each adapter is independently swappable

Composition Root (miniboxd/src/main.rs)
  → Wires adapters to domain via HandlerDependencies
  → Only place that knows about concrete adapter types
```

Rules:
- Domain traits live exclusively in `minibox-core/src/domain.rs`. Never add a trait there that
  imports from `nix`, `libc`, or any platform crate.
- New adapter suites are added to `minibox/src/adapters/` and exported from the module root.
- `HandlerDependencies` is the single struct injected into all request handlers. Adding a field
  requires updating all adapter suite construction sites in `miniboxd/src/main.rs`.
- `Dyn` type aliases (`DynImageRegistry`, etc.) are defined alongside each trait and used in
  `HandlerDependencies` to avoid naming concrete types in handler code.

### ISP sub-structs on HandlerDependencies

`HandlerDependencies` is composed of focused sub-structs so each handler only takes a dependency
on the slice it actually uses (Interface Segregation Principle):

```rust
pub struct HandlerDependencies {
    pub image:     ImageDeps,     // registry routing, image loader, GC, local store
    pub lifecycle: LifecycleDeps, // filesystem, limiter, runtime, state
    pub exec:      ExecDeps,      // exec runtime, PTY registry
    pub build:     BuildDeps,     // image builder
    pub event:     EventDeps,     // event sink, metrics recorder
}
```

When adding a new handler, declare it to accept the narrowest sub-struct it needs — not the full
`HandlerDependencies`. When adding a field to `HandlerDependencies` or a sub-struct, update all
adapter suite construction sites in `miniboxd/src/main.rs`.

### Channel-send failures

Never discard handler channel-send failures with `let _ = tx.send(...)`. Use or follow the
`send_error` helper pattern so dropped client connections appear in logs:

```rust
async fn send_error(tx: &mpsc::Sender<DaemonResponse>, context: &str, message: String) {
    if tx.send(DaemonResponse::Error { message: message.clone() }).await.is_err() {
        warn!(
            context,
            error_message = %message,
            "client disconnected before error response could be sent"
        );
    }
}
```

The same applies to non-error terminal responses — log at `warn!` when the receiver is gone.

### Test doubles

Test doubles live in two locations:

| Location | Scope | Contents |
| -------- | ----- | -------- |
| `minibox-core/src/adapters/mocks.rs` | cross-platform | `MockRegistry`, `MockRuntime`, `MockFilesystem`, `MockLimiter` |
| `minibox/src/testing/` | minibox crate only | `mocks/`, `fixtures/`, `helpers/`, `backend/`, `capability.rs` |

Use `minibox::testing::mocks::*` for handler-level tests that need the full adapter suite.
Use `minibox_core`'s mocks when writing conformance tests for domain traits directly.

Mocks use `Arc<Mutex<…>>` for interior mutability so they can be cloned before injection and
observed from the test after:

```rust
pub struct MockRegistry {
    state: Arc<Mutex<MockRegistryState>>,
}
```

Builder methods configure pre-loaded images and failure modes before injection.

No-op adapters (platforms/tests that don't need a capability) use the `Noop` prefix:
`NoopLimiter`, `NoopEventSink`, `NoopTraceStore`. They return `Ok(())` / empty values.

---

## 6. Platform Gating

Linux-only code is gated with `#[cfg(target_os = "linux")]`. macOS-only code uses
`#[cfg(target_os = "macos")]`. Cross-platform code uses `#[cfg(unix)]`.

```rust
// Module-level gate in adapters/mod.rs
#[cfg(target_os = "linux")]
pub mod filesystem;
#[cfg(target_os = "linux")]
pub use filesystem::OverlayFilesystem;

// Function-level gate
#[cfg(target_os = "linux")]
pub fn warn_if_native_without_root() { ... }
```

For files that are entirely Linux-only, place `#![cfg(target_os = "linux")]` (or `#[cfg(...)]`
on the outer test module) at the top rather than gating each item individually.

```rust
// Integration test file — entirely Linux
#![cfg(target_os = "linux")]
```

**Important:** macOS `cargo check` does not validate `#[cfg(target_os = "linux")]` paths.
Always gate Linux-only imports inside the cfg block, not at the top of the file.

---

## 7. Unsafe Code

Every `unsafe` block must have a `// SAFETY:` comment directly above it explaining:

1. What invariant the caller upholds.
2. Why it cannot be expressed in the type system.

```rust
// SAFETY: We are inside a CLONE_NEWNS child process. The parent has called
// std::mem::forget on all OwnedFds to prevent double-close. This raw fd
// is valid because it was created before clone() and not closed in the parent.
let _ = unsafe { libc::close(read_fd_raw) };
```

For `set_var`/`remove_var` in tests:

```rust
// Serialize env-var-mutating tests to prevent parallel races.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn set_adapter_from_env() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    // SAFETY: env var mutation serialized by ENV_LOCK
    unsafe { std::env::set_var("MINIBOX_ADAPTER", "smolvm"); }
    // ...
    unsafe { std::env::remove_var("MINIBOX_ADAPTER"); }
}
```

---

## 8. Async / Sync Boundary

Container creation, namespace clone, and exec operations must never run inline in `async fn`.
Wrap them in `tokio::task::spawn_blocking`:

```rust
// ✅ Correct
async fn handle_run(&self, req: RunContainer) -> Result<ContainerId> {
    let id = tokio::task::spawn_blocking(move || {
        create_container_namespaces(&req)
    })
    .await
    .context("spawn_blocking join")??;
    Ok(id)
}

// ❌ Wrong — blocks tokio runtime, starves socket accept loop
async fn handle_run(&self, req: RunContainer) -> Result<ContainerId> {
    let id = create_container_namespaces(&req)?;
    Ok(id)
}
```

After a `clone()` or `fork()`, do not use Rust's `OwnedFd` — call `std::mem::forget` on all
owned file descriptors in the parent before the fork to prevent double-close.

---

## 9. Security Invariants

These must never be weakened. Any PR that touches them requires explicit justification.

| Invariant | Location | Rule |
| --------- | -------- | ---- |
| Path traversal | `container/filesystem.rs` | `validate_layer_path()` must be called before any fs op on external paths. Rejects `..` components (fast pre-check) and symlink escapes (canonicalization). |
| Device nodes | `image/layer.rs` | Block and char device tar entries are rejected outright (`ImageError::DeviceNodeRejected`). |
| Absolute symlinks | `image/layer.rs` | Absolute symlink targets are rewritten to relative paths. Targets still containing `..` after rewrite are rejected. |
| Setuid/setgid stripping | `image/layer.rs` | Special permission bits (04000, 02000) are stripped before extraction. |
| Unix socket auth | `miniboxd/src/listener.rs` | `SO_PEERCRED` UID==0 check must run before any request processing. Never bypass. |
| execve environment | `container/process.rs` | Container init uses `execve` with an explicit, minimal env — not `execvp`. |
| Image pull size | `image/registry.rs` | Size limits are enforced during streaming layer download. |

---

## 10. Naming Conventions

### Types and items

| Kind | Convention | Example |
| ---- | ---------- | ------- |
| Structs, enums, traits | `UpperCamelCase` | `ContainerState`, `ImageRegistry` |
| Functions, methods | `snake_case` | `spawn_container_process`, `validate_layer_path` |
| Constants | `UPPER_SNAKE_CASE` | `DAEMON_SOCKET_PATH`, `DEFAULT_ADAPTER_SUITE` |
| Type aliases | `UpperCamelCase` | `DynImageRegistry`, `TraceId` |
| Lifetimes | short lowercase | `'a`, `'buf` |

### Naming patterns by role

- **Domain traits (ports):** noun-based, no verb prefix — `ImageRegistry`, `ResourceLimiter`.
- **Adapters:** `<Technology><PortName>` — `DockerHubRegistry`, `CgroupV2Limiter`,
  `OverlayFilesystem`.
- **Mock adapters:** `Mock<PortName>` — `MockRegistry`, `MockRuntime`.
- **No-op adapters:** `Noop<PortName>` — `NoopLimiter`, `NoopEventSink`.
- **Errors:** `<Subsystem>Error` — `ImageError`, `RegistryError`, `CgroupError`.
- **State tags (typestate):** lifecycle nouns — `Created`, `Running`, `Paused`, `Stopped`.
- **Tests:** descriptive phrases, underscores, no `test_` prefix needed when inside `mod tests` —
  `parse_unknown_returns_structured_error_with_valid_options`.

### Container IDs

Container IDs are the first 12 characters of a UUID v4:

```rust
let id = Uuid::new_v4().to_string().chars().take(12).collect::<String>();
```

---

## 11. Imports and Visibility

### Import ordering

`rustfmt` sorts imports alphabetically within each `use` block. There is no enforced blank-line
grouping between `std`, external crates, and internal crates — `rustfmt` merges them. Local
`crate::` / `super::` imports are typically placed last, separated by a blank line.

```rust
// Typical observed pattern: alphabetical, local at end
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use nix::sys::signal::{Signal, kill};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::container::cgroups::{CgroupConfig, CgroupManager};
use crate::container::filesystem::{cleanup_mounts, setup_overlay};
```

Do not manually reorganize imports into groups — `rustfmt` will reorder them on the next format
pass. Run `cargo fmt` after any import change.

### Visibility

- `pub` — part of the crate's public API.
- `pub(crate)` — needed across modules within the crate, not externally. Used sparingly; most
  items are either fully public or fully private.
- `pub(super)` — needed only by the parent module.
- Private (`fn`, `struct` with no qualifier) is the default for helpers.
- Module structure: `pub mod` for public modules; `mod` for internal modules re-exported via
  `pub use` from the parent.

---

## 12. Section Dividers

Two divider forms are used. Choose the appropriate one by context.

### Unlabeled divider — the standard form

Used everywhere: between logical sections in modules, between test groups, between error type
blocks. This is the dominant form (900+ occurrences).

```rust
// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub enum ContainerState { ... }
```

The rule is `// ` followed by exactly 75 dashes, total line width 78 characters.

### Labeled divider — for large files with many named sections

Used in long files (`handler.rs`, `main.rs`) where a table-of-contents mental model helps. Uses
Unicode box-drawing em-dashes (`─`) and embeds the section name inline.

```rust
// ─── Run ────────────────────────────────────────────────────────────────────

async fn handle_run(...) { ... }

// ─── Stop ───────────────────────────────────────────────────────────────────
```

Use labeled dividers only when the file exceeds ~400 lines and has 5+ distinct top-level
sections. Otherwise use the unlabeled form.

---

## 13. Serialization

### Protocol enums

Use `#[serde(tag = "type")]` for request/response enums so the discriminant appears as an
explicit `"type"` field in the JSON wire format:

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    Run { image: String, tag: Option<String>, ... },
    Stop { container_id: String },
}
// Wire: {"type":"Run","image":"ubuntu","tag":"22.04",...}
```

### Wire compatibility

New fields added to existing protocol variants **must** use `#[serde(default)]`:

```rust
#[derive(Serialize, Deserialize)]
pub struct RunRequest {
    pub image: String,
    #[serde(default)]           // wire-compatible: missing field deserializes as None
    pub session_id: Option<String>,
}
```

### Rename conventions

Enum variant names in JSON use `rename_all = "snake_case"` for event types:

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContainerEvent {
    Created { ... },   // → "type":"created"
    OomKilled { ... }, // → "type":"oom_killed"
}
```

---

## 14. Testing

### Test structure

All modules include a `#[cfg(test)] mod tests { ... }` block, even for Linux-only code. Tests that
require Linux are individually gated:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Cross-platform unit test — always runs
    #[test]
    fn parse_unknown_returns_structured_error() { ... }

    // Linux-only test
    #[test]
    #[cfg(target_os = "linux")]
    fn parse_native_succeeds() { ... }
}
```

### Test categories

| Category | Location | Requirements |
| -------- | -------- | ------------ |
| Unit | inline `mod tests` | None. In-memory only. |
| Conformance | `crates/*/tests/conformance_*.rs` | None. Uses mock adapters. |
| Property | `tests/proptest_suite.rs` | None. `proptest` crate. |
| Integration | `tests/integration_tests.rs` | Linux + root + network. |
| E2E | `tests/system_tests.rs` | Linux + root + running daemon. |

Run with: `cargo xtask test-unit` (cross-platform), `just test-integration` (Linux+root).

### Test double usage

Use `MockRegistry`, `MockRuntime`, etc. from `adapters::mocks` for handler unit tests. Never
construct real adapters (cgroups, overlay, network) in tests that run on macOS.

### Snapshot tests

Snapshot tests use `insta`. Run `cargo insta review` to accept new snapshots. Snapshots live in
`tests/snapshots/` and are committed.

### Naming

Tests are named with descriptive phrases that read as specifications:

```rust
fn parse_unknown_returns_structured_error_with_valid_options() { ... }
fn compiled_adapters_matches_available_adapter_names() { ... }
fn linux_only_adapters_are_unavailable_on_non_linux() { ... }
```

### `expect` vs `unwrap` in tests

Use `expect("reason")` instead of `unwrap()` in tests. The reason string explains what the test
expects:

```rust
parse_adapter("native").expect("should parse native");
adapter_from_env().expect("default adapter should parse on any unix platform");
```

---

## 15. Macros

Macros live in `minibox-macros/` and are declared with `macro_rules!`. Key macros:

| Macro | Purpose |
| ----- | ------- |
| `as_any!` | Implement `AsAny` (downcast support) for one or more types |
| `default_new!` | Implement `Default` via `Self::new()` |
| `adapt!` | Implement both `AsAny` and `Default` |
| `require_capability!` | Skip a test when a host capability is absent (cgroups, overlay, etc.) |
| `normalize!` | Replace `/` and `:` with `_` for filesystem path components |
| `test_run!` | Construct a default `DaemonRequest::Run` for tests |

**`crate` vs `$crate` gotcha:** `as_any!` references `crate::domain::AsAny`. In `macro_rules!`,
`crate` resolves at the call site, not the defining crate. Do not change it to `$crate` — that
would resolve to `minibox_macros` which does not define `AsAny`. The `#[allow(clippy::crate_in_macro_def)]`
suppression is intentional.

---

## 16. Typestate Pattern

`crates/minibox-core/src/typestate.rs` encodes the container lifecycle in the type system using
the typestate pattern. State tags are zero-sized structs; `Container<S>` is generic over the
state tag. Transition methods consume `self` and return the container in its new state.

```rust
pub struct Created;
pub struct Running { pub pid: u32 }
pub struct Stopped { pub exit_code: i32 }

impl Container<Created> {
    pub fn start(self, ...) -> Result<Container<Running>> { ... }
}
impl Container<Running> {
    pub fn stop(self, ...) -> Result<Container<Stopped>> { ... }
    pub fn pause(self, ...) -> Result<Container<Paused>> { ... }
}
```

Use this pattern for any new protocol-level state machine where invalid transitions should be
caught at compile time rather than checked at runtime.

---

## 17. Protocol Changes

Protocol types are canonical in `crates/minibox-core/src/protocol.rs`. When adding or modifying
protocol types:

1. **Add/modify the type** in `protocol.rs`.
2. **Add `#[serde(default)]`** on any new optional field for wire compatibility.
3. **Update handlers** in `minibox/src/daemon/handler.rs` that pattern-match on the changed variant.
4. **Update CLI** `mbx/src/commands/` for the corresponding subcommand.
5. **Update snapshot tests** — protocol evolution tests live in
   `crates/minibox-core/tests/protocol_evolution.rs`.
6. **Run `cargo xtask verify`** to confirm fmt, clippy, and borrow fixtures pass.

`DaemonResponse::ContainerOutput` is **non-terminal** (a container can produce many output chunks
before stopping). All other response variants end request streaming. When adding a new response
variant, explicitly decide and document whether it is terminal or non-terminal, and update the
terminal-response logic in the handler.

---

## Quick Reference

```
No .unwrap() in production        → use .context("description")?
No println!/eprintln! in daemon   → use tracing::info!/warn!
No platform imports in core       → minibox-core has zero OS deps
No fork/clone in async fn         → use tokio::task::spawn_blocking
No unsafe without SAFETY comment  → document the invariant
No direct path from user input    → call validate_layer_path() first
No env::set_var in parallel tests → use static Mutex<()> guard
No new protocol field without     → #[serde(default)]
  backward compat
New adapter? Update composition   → miniboxd/src/main.rs (all suites)
New HandlerDependencies field?    → update all construction sites
```
