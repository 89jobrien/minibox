---
title: WSL2 Executor Injection Seam
status: approved
date: 2026-03-20
---

# WSL2 Executor Injection Seam

## Goal

Add an injectable executor to the WSL2 adapter structs so they can be unit-tested without a real WSL2 installation, following the proven LimaExecutor pattern from colima.rs.

## Scope

crates/minibox-lib/src/adapters/wsl2.rs only. Docker Desktop adapter is out of scope.

## Design

### Executor Type Alias

```rust
type WslExecutor = Arc<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;
```

Identical shape to LimaExecutor. Defined at the top of wsl2.rs.

### Struct Changes

Add `executor: Option<WslExecutor>` to each of the three adapter structs:

- Wsl2Runtime
- Wsl2Filesystem
- Wsl2Limiter

Each struct also gets a `with_executor(executor: WslExecutor) -> Self` builder method.

### Exec Method Update

wsl_exec gains an early-return path:

```rust
fn wsl_exec(&self, args: &[&str]) -> Result<String> {
    if let Some(exec) = &self.executor {
        return exec(args);
    }
    // existing Command::new("wsl.exe") path unchanged
}
```

All three structs follow this same pattern for their respective exec methods.

### Tests

Added in #[cfg(test)] block at the bottom of wsl2.rs. Each test:
1. Constructs the adapter with with_executor(Arc::new(|args| { ... }))
2. Calls a domain trait method
3. Asserts on the result or captured args

Minimum coverage:
- Wsl2Runtime: run_container with fake executor returning success
- Wsl2Filesystem: setup_container_fs with fake executor
- Wsl2Limiter: apply_limits with fake executor

## What Does Not Change

- Production code paths (the Command::new("wsl.exe") branch is untouched)
- Existing Default / new() constructors -- executor defaults to None
- adapt!() / as_any!() macro usage
- Docker Desktop adapter
