# Rust Patterns — Minibox Development Rules

Minibox-specific Rust idioms and constraints. Applied to all code in this repository.

## Non-Negotiable Minibox Rules

These override general Rust conventions:

1. **No `.unwrap()` in production** — Use `.into_diagnostic().wrap_err("description")?` (or `.wrap_err("description")?` directly for miette-native error types). Tests: use `expect("reason")`.
2. **Path validation on all user input** — Every path derived from user input or external data (tar entries, image refs, CLI args) must go through `validate_layer_path()` or equivalent canonicalize+prefix-check before touching the filesystem.
3. **`spawn_blocking` for fork/clone/exec** — Container creation operations must not run inline in `async fn`. Always wrap in `tokio::task::spawn_blocking`.
4. **`SO_PEERCRED` auth is mandatory** — The UID==0 check in `minibox/src/daemon/server.rs` must run before any request processing. Never bypass or weaken it.
5. **Tracing structured fields** — Use `key = value` syntax in `tracing::info!/warn!/error!/debug!` macros. Never embed structured values in the message string.
6. **`unsafe` blocks require documented invariants** — Every `unsafe {}` must have a comment explaining what invariant the caller upholds and why it cannot be expressed in the type system.

## Mutex Guard Binding

`let _ = mutex.lock()` drops the guard immediately (no critical section held) —
workspace clippy denies this. Bind to a named variable instead:
`let _state = mutex.lock()...` (or an explicit `.expect(...)` if fallible).

## Error Handling

### Always context, always miette

`anyhow` is deprecated in this repo. New code and any code touched during a refactor must use
`miette` (`Result<T>`, `IntoDiagnostic`, `WrapErr`/`Context`) instead. Migration is in progress —
see the migration note below.

```rust
use miette::{IntoDiagnostic, Result, WrapErr};

// ✅ Correct
fn read_manifest(path: &Path) -> Result<ImageManifest> {
    let content = fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to read manifest: {}", path.display()))?;
    serde_json::from_str(&content)
        .into_diagnostic()
        .wrap_err("Failed to parse image manifest JSON")
}

// ❌ Wrong — no context
fn read_manifest(path: &Path) -> Result<ImageManifest> {
    let content = fs::read_to_string(path).into_diagnostic()?;
    Ok(serde_json::from_str(&content).into_diagnostic()?)
}

// ❌ Wrong — panic in daemon crashes all containers
fn read_manifest(path: &Path) -> ImageManifest {
    let content = fs::read_to_string(path).unwrap();
    serde_json::from_str(&content).unwrap()
}
```

For error types that are matched on (protocol errors, adapter errors, domain errors), keep
`thiserror` enums and derive `miette::Diagnostic` on them for rich CLI rendering (error codes,
help text, source spans) instead of collapsing them into an opaque report at the point of origin.

#### Migration note

`anyhow` still appears throughout the daemon/core/adapters. Do not do a mechanical
find-and-replace across the workspace in one pass — convert a module at a time, verify
`cargo check -p <crate>` after each, and keep the crate's public error type stable across the
boundary (callers outside the crate shouldn't need to know whether it's using anyhow or miette
internally mid-migration). `minibox-cli` already renders errors via miette (`cf37b05a`); that's
the reference pattern for `.wrap_err()` usage and `Diagnostic` derives.

### Cleanup on failure (mandatory for resource-creating functions)

```rust
// ✅ Correct: clean up overlay mount if cgroup setup fails
fn create_container(config: &ContainerConfig) -> Result<ContainerId> {
    let rootfs = create_overlay(&config.layers, &id)
        .wrap_err("create_overlay")?;

    if let Err(e) = setup_cgroup(&id, &config.limits) {
        // Best-effort cleanup — log warn, don't propagate secondary error
        if let Err(cleanup_err) = destroy_overlay(&id) {
            tracing::warn!(
                container_id = %id,
                error = %cleanup_err,
                "container: overlay cleanup failed after cgroup error"
            );
        }
        return Err(e).wrap_err("setup_cgroup");
    }
    Ok(id)
}
```

## Path Validation — Mandatory for All External Paths

```rust
// ✅ Correct: validate before any filesystem operation
fn extract_entry(entry: &TarEntry, dest: &Path) -> Result<()> {
    let entry_path = entry.path().into_diagnostic().wrap_err("entry path")?;
    validate_layer_path(&entry_path)?;  // Rejects .., absolute paths

    let target = dest.join(&entry_path);
    // Canonicalize parent to catch symlink-based traversal
    let parent = target.parent().unwrap_or(dest);
    if parent.exists() {
        let canonical = fs::canonicalize(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("canonicalize {}", parent.display()))?;
        if !canonical.starts_with(dest) {
            miette::bail!("path escapes destination: {}", entry_path.display());
        }
    }
    // Safe to write
}

// ❌ Wrong: direct join without validation
fn extract_entry(entry: &TarEntry, dest: &Path) -> Result<()> {
    let target = dest.join(entry.path().into_diagnostic()?);
    fs::write(&target, data)?;  // Zip Slip if path is "../../../etc/cron.d/evil"
}
```

## Async/Sync Boundary

```rust
// ✅ Correct: container operations in spawn_blocking
async fn handle_run(
    &self,
    req: RunContainer,
    state: Arc<Mutex<DaemonState>>,
) -> Result<ContainerId> {
    let id = tokio::task::spawn_blocking(move || {
        create_container_namespaces(&req)
    })
    .await
    .into_diagnostic()
    .wrap_err("spawn_blocking join")??;

    state.lock().await.add_container(id.clone(), ContainerRecord::new(&req));
    Ok(id)
}

// ❌ Wrong: blocks tokio worker — starves socket accept loop
async fn handle_run(&self, req: RunContainer) -> Result<ContainerId> {
    let id = create_container_namespaces(&req)?;  // clone() blocks entire runtime!
    Ok(id)
}
```

## Tracing — Structured Fields Only

```rust
// ✅ Correct: key = value fields, lowercase verb-noun message
tracing::info!(
    container_id = %id,
    pid = pid.as_raw(),
    rootfs = %config.rootfs.display(),
    "container: process started"
);

tracing::warn!(
    entry = %entry.display(),
    target = %symlink_target.display(),
    "tar: rejected absolute symlink"
);

// ❌ Wrong: values embedded in message string (not queryable)
tracing::info!("Container {} started with PID {}", id, pid);
tracing::warn!("Rejected symlink {} -> {}", entry.display(), target.display());
```

### Tracing severity discipline

| Level    | Usage                                                                                    |
| -------- | ---------------------------------------------------------------------------------------- |
| `error!` | Unrecoverable: container init crash, fatal exec error, daemon cannot continue            |
| `warn!`  | Security rejections, degraded behaviour, best-effort cleanup failures                    |
| `info!`  | Lifecycle milestones: container start/stop, image pull phases, overlay mount, pivot_root |
| `debug!` | Syscall arguments, byte counts, internal state transitions                               |

## Unsafe Blocks

```rust
// ✅ Correct: document the invariant
// SAFETY: We are inside a CLONE_NEWNS child process. The parent has called
// std::mem::forget on all OwnedFds to prevent double-close. This raw fd
// is valid because it was created before clone() and not closed in the parent.
let _ = unsafe { libc::close(read_fd_raw) };

// ❌ Wrong: no invariant documented
let _ = unsafe { libc::close(read_fd_raw) };
```

## Ownership — Borrow Over Clone

```rust
// ✅ Prefer borrows in processing functions
fn filter_log_lines<'a>(input: &'a str) -> Vec<&'a str> {
    input.lines()
        .filter(|line| !line.is_empty())
        .collect()
}

// ✅ Clone only when ownership is genuinely required
fn build_overlay_options(layers: &[PathBuf]) -> String {
    let lowerdir = layers.iter()
        .map(|p| p.to_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(":");
    format!("lowerdir={}", lowerdir)
}

// ❌ Unnecessary clone in hot path
fn build_overlay_options(layers: &[PathBuf]) -> String {
    let owned: Vec<PathBuf> = layers.to_vec();  // Clone for no reason
    // ...
}
```

## Iterators Over Loops

```rust
// ✅ Iterator chain — idiomatic
let layer_paths: Vec<PathBuf> = manifest.layers
    .iter()
    .map(|layer| image_dir.join(&layer.digest))
    .collect();

// ✅ Use find/filter/map for processing
let running: Vec<_> = state.containers
    .values()
    .filter(|c| c.status == ContainerStatus::Running)
    .collect();
```

## Module Structure Conventions

### Container module files follow this pattern:

```rust
// 1. Imports
use miette::{IntoDiagnostic, Result, WrapErr};
use nix::sched::CloneFlags;

// 2. Public types
pub struct OverlayMount { ... }

// 3. Public entry point(s)
pub fn create_overlay(layers: &[PathBuf], id: &ContainerId) -> Result<OverlayMount> { ... }

// 4. Private helpers
fn build_mount_options(layers: &[PathBuf]) -> String { ... }

// 5. Tests (always present, even for Linux-only code)
#[cfg(test)]
mod tests {
    use super::*;
    // Unit tests using mock paths / in-memory data
    // Linux-only tests gated with #[cfg(target_os = "linux")]
}
```

### Adapter module pattern:

```rust
pub struct MyPlatformRuntime {
    // adapter-specific state
}

impl ContainerRuntime for MyPlatformRuntime {
    fn create(&self, config: &ContainerConfig) -> Result<ContainerHandle> {
        // platform-specific implementation
    }
    // ...
}

// Tests: use mock adapters from adapters::mocks
```

## Rustqual Lint Suppressions

Workspace-wide SRP/complexity lints are enforced via the `rustqual` tool (see the ongoing
"Rustqual SRP sweep"). When a function genuinely can't be split further — an I/O-boundary
orchestration function, or an infallible/irrecoverable operation — suppress with an explicit
code and reason, immediately above the item:

```rust
// qual:allow(iosp) reason: "daemon bootstrap: config/logging/adapter selection + side-effectful initialization"
async fn run_daemon(config: miniboxd::config::DaemonConfig) -> Result<()> { ... }

// qual:allow(complexity) reason: "poisoned mutex is irrecoverable"
pub fn remove(&self) -> Result<()> { ... }

// qual:allow(complexity, iosp) reason: "bridge network setup: veth pair, IP alloc, iptables"
async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> { ... }
```

Known codes: `iosp` (I/O-boundary orchestration — a function that necessarily coordinates several
side effects: bind socket, spawn process, stream output), `complexity` (branching/control-flow
that can't be reduced without obscuring intent, e.g. a poisoned-mutex-is-fatal path).

**Never suppress without a reason string.** The reason is the reviewer's justification for
accepting the exception — write it as if defending the choice, not describing what the code does.

### Extract, don't suppress, when a function grows for no structural reason

Before reaching for `qual:allow`, check whether the function is doing one job awkwardly or
several jobs stitched together. If it's the latter, extract named helpers — this repo's
convention is `verb_noun` helpers pulled out in place, right above or below the caller in the
same file, not moved to a new module:

```rust
// ✅ Before: run_daemon() inlined signal handling
// After: extracted with a name that states exactly what it does
fn install_shutdown_signal_handlers() -> Result<impl std::future::Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = signal(SignalKind::terminate()).wrap_err("SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).wrap_err("SIGINT handler")?;
    Ok(async move {
        tokio::select! {
            _ = sigterm.recv() => { info!("received SIGTERM, shutting down"); }
            _ = sigint.recv()  => { info!("received SIGINT, shutting down");  }
        }
    })
}
```

Only reach for `qual:allow` once extraction genuinely doesn't apply — e.g. the function's whole
job _is_ orchestration (bootstrap, setup/teardown sequences) and splitting it would scatter
sequencing logic across files without reducing complexity.

## Clippy Allows in Test Modules

Workspace lints deny `unwrap_used`/`expect_used`/`panic` across **all** targets, including tests
— `cargo clippy --all-targets --workspace -- -D warnings` fails on a bare `#[cfg(test)] mod
tests` that uses `.unwrap()`/`.expect()`/`panic!()`, which is otherwise idiomatic in test code.
Add the allow directly on the test module, not workspace-wide:

```rust
#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::default_constructed_unit_structs
)]
mod tests {
    use super::*;
    // ...
}
```

Do this per-module as tests are added — don't pre-emptively blanket-allow at the crate level.

## Anti-Patterns (Minibox-Specific)

| Pattern                                       | Problem                                     | Fix                                           |
| --------------------------------------------- | ------------------------------------------- | --------------------------------------------- |
| `.unwrap()` in production                     | Daemon panic orphans all containers         | `.into_diagnostic().wrap_err()?`              |
| `Path::join(user_input)` without validation   | Zip Slip / path traversal                   | `validate_layer_path()` first                 |
| `fork()`/`clone()` in async fn                | Blocks tokio runtime, possible UB           | `tokio::task::spawn_blocking`                 |
| `println!` in daemon code                     | Contaminates container stdio                | `tracing::info!/warn!`                        |
| Embedded values in tracing message            | Not queryable in log aggregators            | `key = value` structured fields               |
| `unsafe` without SAFETY comment               | Reviewer can't verify correctness           | Document invariant                            |
| Absolute symlink written without rewrite      | Host path leak after pivot_root             | `relative_path()` rewrite                     |
| Missing cleanup on error path                 | Orphaned cgroups, stuck overlays            | Explicit cleanup with warn on secondary error |
| `set_var`/`remove_var` in tests without mutex | Parallel test races                         | `static Mutex<()>` guard                      |
| `OwnedFd` alive across `clone()`              | Double-close in parent and child            | `std::mem::forget` before clone               |
| `.ok()` swallowing a fallible call            | Silent failure, e.g. network never attached | Propagate with `.wrap_err(...)?`              |
| `format!("{val:?}")` on an enum for display   | Breaks if variant names/Debug repr change   | Add/use `as_str()` or `Display`               |
