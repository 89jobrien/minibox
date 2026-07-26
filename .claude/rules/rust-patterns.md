# Rust Patterns — Minibox Development Rules

Minibox-specific Rust idioms and constraints. Applied to all code in this repository.

## Non-Negotiable Minibox Rules

These override general Rust conventions:

1. **No `.unwrap()` in production** — Use `.context("description")?`. Tests: use `expect("reason")`.
2. **Path validation on all user input** — Every path derived from user input or external data (tar entries, image refs, CLI args) must go through `validate_layer_path()` or equivalent canonicalize+prefix-check before touching the filesystem.
3. **`spawn_blocking` for fork/clone/exec** — Container creation operations must not run inline in `async fn`. Always wrap in `tokio::task::spawn_blocking`.
4. **`SO_PEERCRED` auth is mandatory** — The UID==0 check in `minibox/src/daemon/server.rs` must run before any request processing. Never bypass or weaken it.
5. **Tracing structured fields** — Use `key = value` syntax in `tracing::info!/warn!/error!/debug!` macros. Never embed structured values in the message string.
6. **`unsafe` blocks require documented invariants** — Every `unsafe {}` must have a comment explaining what invariant the caller upholds and why it cannot be expressed in the type system.

## Error Handling

### Always context, always anyhow

```rust
use anyhow::{Context, Result};

// ✅ Correct
fn read_manifest(path: &Path) -> Result<ImageManifest> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read manifest: {}", path.display()))?;
    serde_json::from_str(&content)
        .context("Failed to parse image manifest JSON")
}

// ❌ Wrong — no context
fn read_manifest(path: &Path) -> Result<ImageManifest> {
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

// ❌ Wrong — panic in daemon crashes all containers
fn read_manifest(path: &Path) -> ImageManifest {
    let content = fs::read_to_string(path).unwrap();
    serde_json::from_str(&content).unwrap()
}
```

### Cleanup on failure (mandatory for resource-creating functions)

```rust
// ✅ Correct: clean up overlay mount if cgroup setup fails
fn create_container(config: &ContainerConfig) -> Result<ContainerId> {
    let rootfs = create_overlay(&config.layers, &id)
        .context("create_overlay")?;

    if let Err(e) = setup_cgroup(&id, &config.limits) {
        // Best-effort cleanup — log warn, don't propagate secondary error
        if let Err(cleanup_err) = destroy_overlay(&id) {
            tracing::warn!(
                container_id = %id,
                error = %cleanup_err,
                "container: overlay cleanup failed after cgroup error"
            );
        }
        return Err(e).context("setup_cgroup");
    }
    Ok(id)
}
```

## Path Validation — Mandatory for All External Paths

```rust
// ✅ Correct: validate before any filesystem operation
fn extract_entry(entry: &TarEntry, dest: &Path) -> Result<()> {
    let entry_path = entry.path().context("entry path")?;
    validate_layer_path(&entry_path)?;  // Rejects .., absolute paths

    let target = dest.join(&entry_path);
    // Canonicalize parent to catch symlink-based traversal
    let parent = target.parent().unwrap_or(dest);
    if parent.exists() {
        let canonical = fs::canonicalize(parent)
            .with_context(|| format!("canonicalize {}", parent.display()))?;
        if !canonical.starts_with(dest) {
            bail!("path escapes destination: {}", entry_path.display());
        }
    }
    // Safe to write
}

// ❌ Wrong: direct join without validation
fn extract_entry(entry: &TarEntry, dest: &Path) -> Result<()> {
    let target = dest.join(entry.path()?);
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
    .context("spawn_blocking join")??;

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
use anyhow::{Context, Result};
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

## Anti-Patterns (Minibox-Specific)

| Pattern                                       | Problem                             | Fix                                           |
| --------------------------------------------- | ----------------------------------- | --------------------------------------------- |
| `.unwrap()` in production                     | Daemon panic orphans all containers | `.context()?`                                 |
| `Path::join(user_input)` without validation   | Zip Slip / path traversal           | `validate_layer_path()` first                 |
| `fork()`/`clone()` in async fn                | Blocks tokio runtime, possible UB   | `tokio::task::spawn_blocking`                 |
| `println!` in daemon code                     | Contaminates container stdio        | `tracing::info!/warn!`                        |
| Embedded values in tracing message            | Not queryable in log aggregators    | `key = value` structured fields               |
| `unsafe` without SAFETY comment               | Reviewer can't verify correctness   | Document invariant                            |
| Absolute symlink written without rewrite      | Host path leak after pivot_root     | `relative_path()` rewrite                     |
| Missing cleanup on error path                 | Orphaned cgroups, stuck overlays    | Explicit cleanup with warn on secondary error |
| `set_var`/`remove_var` in tests without mutex | Parallel test races                 | `static Mutex<()>` guard                      |
| `OwnedFd` alive across `clone()`              | Double-close in parent and child    | `std::mem::forget` before clone               |
