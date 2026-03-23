---
title: WSL2 Executor Injection Seam
status: approved
date: 2026-03-20
---

# WSL2 Executor Injection Seam

## Goal

Add an injectable executor to the WSL2 adapter so it can be unit-tested without a real WSL2
installation, following the proven LimaExecutor pattern from colima.rs.

## Scope

`crates/linuxbox/src/adapters/wsl2.rs` only. Docker Desktop adapter is out of scope.

## Structural Observation

`Wsl2Filesystem` and `Wsl2Limiter` own a `runtime: Wsl2Runtime` field and delegate all
subprocess calls through `self.runtime.wsl_exec()`. They have no independent exec method.
The executor therefore lives only on `Wsl2Runtime`; the outer structs gain `with_executor`
builders that delegate into their inner runtime.

`spawn_process` bypasses `wsl_exec()` — it constructs its own `Command` inline inside
`spawn_blocking`. It must be updated separately to check the injected executor before
reaching `spawn_blocking`.

## Design

### Executor Type Alias

```rust
type WslExecutor = Arc<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;
```

The executor receives WSL-side args only — no `wsl.exe`, `-d <distro>`, or `--` prefix.
This matches the convention of `wsl_exec`, which wraps `Command::new("wsl.exe")` and
accepts only the inner args.

`Arc<dyn Fn(...)>` is `Clone`; the existing `#[derive(Clone)]` on all three structs is
unaffected.

### Wsl2Runtime Changes

```rust
pub struct Wsl2Runtime {
    distro: String,
    helper_path: String,
    executor: Option<WslExecutor>,   // new
}

pub fn with_executor(mut self, executor: WslExecutor) -> Self {
    self.executor = Some(executor);
    self
}
```

`wsl_exec` gains early-return — the production `Command::new("wsl.exe")` branch is
unchanged:

```rust
fn wsl_exec(&self, args: &[&str]) -> Result<String> {
    if let Some(exec) = &self.executor {
        return exec(args);
    }
    // existing Command::new("wsl.exe") path unchanged
}
```

`spawn_process` gains an early-return that replaces the `spawn_blocking` block entirely.
The path-conversion calls (`windows_to_wsl_path`) remain above the branch and already
go through `wsl_exec`, so they are automatically intercepted by the executor:

```rust
async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
    // Path conversion happens BEFORE the executor branch.
    // Each call goes through wsl_exec and is intercepted by the executor.
    let rootfs_wsl = self.windows_to_wsl_path(&config.rootfs)?;
    let cgroup_path_wsl = self.windows_to_wsl_path(&config.cgroup_path)?;

    let json = serde_json::to_string(&spawn_request)?;

    // --- injected path: replaces the entire spawn_blocking block ---
    if let Some(exec) = &self.executor {
        let args: Vec<&str> = vec!["sudo", &self.helper_path, "spawn", &json];
        let stdout = exec(&args)?;
        let response: WslSpawnResponse = serde_json::from_str(&stdout)?;
        return Ok(SpawnResult { pid: response.pid, output_reader: None });
    }

    // --- production path: spawn_blocking block unchanged ---
    let distro = self.distro.clone();
    let helper_path = self.helper_path.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new("wsl.exe")
            .arg("-d").arg(&distro).arg("--")
            .arg("sudo").arg(&helper_path).arg("spawn").arg(&json)
            .output()
    })
    .await?
    .context("failed to execute WSL helper")?;
    // ... rest unchanged
}
```

### Wsl2Filesystem and Wsl2Limiter Changes

Each gains a `with_executor` builder that replaces the inner runtime. No other change
is needed — all exec delegation already flows through `self.runtime.wsl_exec()`:

```rust
pub fn with_executor(mut self, executor: WslExecutor) -> Self {
    self.runtime = self.runtime.with_executor(executor);
    self
}
```

### Tests

Added in `#[cfg(test)]` at the bottom of `wsl2.rs`. Correct trait method names:

| Struct | Method under test | Annotation |
|---|---|---|
| `Wsl2Runtime` | `spawn_process` (async) | `#[tokio::test]` |
| `Wsl2Filesystem` | `setup_rootfs` | `#[test]` |
| `Wsl2Limiter` | `create` | `#[test]` |

For the `spawn_process` test, the executor must handle three calls in order:
1. `["wslpath", "-u", <rootfs_path>]` — from `windows_to_wsl_path` for rootfs
2. `["wslpath", "-u", <cgroup_path>]` — from `windows_to_wsl_path` for cgroup_path
3. `["sudo", <helper_path>, "spawn", <json>]` — the actual spawn call

`wslpath` calls return a fake path string; the `spawn` call returns a JSON string
deserializable as `WslSpawnResponse { pid: <n> }`.

`setup_rootfs` and `create` tests follow the same pattern but with simpler
executor shapes (no `wslpath` calls — test input uses pre-converted WSL paths via
direct struct field control or by having the executor handle them).

## What Does Not Change

- Production code paths (both `wsl_exec` fallback and `spawn_blocking` block are
  preserved untouched)
- `new()` constructors — `executor` defaults to `None`
- `as_any!()` macro invocation at the bottom of the file
- Docker Desktop adapter
