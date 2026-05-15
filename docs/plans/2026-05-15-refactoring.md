# Plan: Workspace Refactoring — Domain Nix Cleanup, Fixture Consolidation, Handler Split

## Goal

Remove OS-specific logic from the domain layer (#354), eliminate duplicated test
fixtures (#355), and split the 4,264-line `handler.rs` into navigable feature-cluster
modules (#356) — all without changing observable behaviour.

## Architecture

- Crates affected: `minibox-core`, `minibox`
- New modules: `crates/minibox/src/daemon/handlers/` (9 files + `mod.rs`)
- No new traits or types — pure structural refactoring
- Data flow unchanged — `server.rs` dispatch table imports via re-exports

## Tech Stack

- Rust 2024, `anyhow`, `nix` (already a `minibox` dependency), `tokio`
- No new dependencies

## Tasks

### Task 1: Remove nix waitpid from minibox-core domain trait default

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs`
**Run**: `cargo check -p minibox-core`

1. Write failing test — compile-time verification that `nix` is not used in domain:

   ```rust
   // crates/minibox-core/src/domain.rs, inside existing test module or new one
   #[cfg(test)]
   mod domain_purity_tests {
       #[test]
       fn wait_for_exit_default_returns_error_on_non_unix() {
           // The default impl should return Err, not silently Ok(-1).
           // This test documents the new contract.
       }
   }
   ```

   Run: `cargo nextest run -p minibox-core -- domain_purity`
   Expected: FAIL (test doesn't exist yet)

2. Implement — replace the `wait_for_exit` default impl (lines 915–943 in
   `crates/minibox-core/src/domain.rs`):

   ```rust
   async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> Result<i32> {
       anyhow::bail!(
           "wait_for_exit: no default implementation — \
            adapter must override with platform-specific wait logic"
       )
   }
   ```

   Remove the `#[cfg(unix)]` / `#[cfg(not(unix))]` blocks and the `nix` imports
   (`nix::sys::wait::{WaitStatus, waitpid}`, `nix::unistd::Pid`).

3. Verify:

   ```
   cargo check -p minibox-core              -> OK (no nix dependency)
   cargo clippy -p minibox-core -- -D warnings  -> zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output matches the expected branch. Stop immediately if not.
   Commit: `git commit -m "refactor(minibox-core): remove nix waitpid from domain trait default (#354)"`

### Task 2: Remove nix waitpid from minibox domain re-export

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/domain.rs`
**Run**: `cargo check -p minibox`

1. The `minibox` crate's `domain.rs` (line 519) has an identical `wait_for_exit`
   default impl with `nix` imports. Apply the same change as Task 1:

   Replace lines 519–547 in `crates/minibox/src/domain.rs` with:

   ```rust
   async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> Result<i32> {
       anyhow::bail!(
           "wait_for_exit: no default implementation — \
            adapter must override with platform-specific wait logic"
       )
   }
   ```

2. Verify:

   ```
   cargo check -p minibox                   -> OK
   cargo clippy -p minibox -- -D warnings   -> zero warnings
   ```

3. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(minibox): remove nix waitpid from domain re-export (#354)"`

### Task 3: Add wait_for_exit override to LinuxNamespaceRuntime

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/runtime.rs`
**Run**: `cargo nextest run -p minibox`

1. Write failing test — call `wait_for_exit` on `LinuxNamespaceRuntime` to confirm
   the override is present (before adding it, the test will fail because the default
   now returns `Err`):

   ```rust
   #[cfg(test)]
   mod wait_for_exit_tests {
       use super::*;

       #[tokio::test]
       async fn linux_runtime_overrides_wait_for_exit() {
           let rt = LinuxNamespaceRuntime;
           // PID 1 (init) is always running — waitpid on it will return
           // ECHILD since we are not its parent, but the point is that
           // the method does NOT return the default bail!() error message.
           let result = rt.wait_for_exit(None, 1).await;
           // Should NOT contain "no default implementation"
           if let Err(e) = &result {
               assert!(
                   !format!("{e:?}").contains("no default implementation"),
                   "LinuxNamespaceRuntime must override wait_for_exit, not use default"
               );
           }
       }
   }
   ```

   Run: `cargo nextest run -p minibox -- linux_runtime_overrides_wait`
   Expected: FAIL (default returns bail)

2. Implement — add `wait_for_exit` override to the `ContainerRuntime` impl for
   `LinuxNamespaceRuntime` in `crates/minibox/src/adapters/runtime.rs`:

   ```rust
   async fn wait_for_exit(&self, _runtime_id: Option<&str>, pid: u32) -> Result<i32> {
       tokio::task::spawn_blocking(move || {
           crate::container::process::wait_for_exit(pid)
       })
       .await
       .context("wait_for_exit: join error")?
   }
   ```

   This delegates to the existing free function in `container/process.rs:593` which
   already handles `waitpid`, `WaitStatus` matching, and structured logging.

3. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   cargo clippy -p minibox -- -D warnings    -> zero warnings
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "feat(minibox): add wait_for_exit override to LinuxNamespaceRuntime (#354)"`

### Task 4: Add wait_for_exit override to MockRuntime variants

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/mocks.rs`
**Run**: `cargo nextest run -p minibox`

1. The `MockRuntime` in `minibox` likely relies on the trait default. After Task 1–2,
   handler tests calling `daemon_wait_for_exit` will fail because MockRuntime now
   hits the `bail!()` default.

   Run: `cargo nextest run -p minibox`
   Expected: failures in handler tests that exercise wait paths

2. Implement — add `wait_for_exit` override to `MockRuntime`'s `ContainerRuntime`
   impl in `crates/minibox/src/adapters/mocks.rs`:

   ```rust
   async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> Result<i32> {
       Ok(0)
   }
   ```

   Also add the same override to the `minibox-core` mock in
   `crates/minibox-core/src/adapters/mocks.rs` if it has a `ContainerRuntime` impl:

   ```rust
   async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> Result<i32> {
       Ok(0)
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox -p minibox-core  -> all green
   cargo clippy --workspace -- -D warnings       -> zero warnings
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "fix(mocks): add wait_for_exit override to MockRuntime variants (#354)"`

### Task 5: Delete duplicate test_fixtures.rs, add re-export

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/test_fixtures.rs`, `crates/minibox/src/adapters/mod.rs`
**Run**: `cargo nextest run -p minibox`

1. Verify duplication — confirm the two `test_fixtures.rs` files are structurally
   identical (only import paths differ). The `minibox` version uses
   `use crate::domain::` while `minibox-core` uses `use minibox_core::domain::`.
   Since `minibox` re-exports `minibox_core::domain`, both resolve to the same types.

2. Implement:

   a. Delete `crates/minibox/src/adapters/test_fixtures.rs`.

   b. In `crates/minibox/src/adapters/mod.rs`, replace lines 133–134:

      ```rust
      // Before:
      #[cfg(test)]
      pub mod test_fixtures;

      // After:
      #[cfg(test)]
      pub use minibox_core::adapters::test_fixtures;
      ```

3. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   cargo clippy -p minibox -- -D warnings    -> zero warnings
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(minibox): consolidate test_fixtures via re-export (#355)"`

### Task 6: Create handlers/ module directory with common.rs

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handlers/mod.rs`,
             `crates/minibox/src/daemon/handlers/common.rs`
**Run**: `cargo check -p minibox`

1. Create `crates/minibox/src/daemon/handlers/mod.rs`:

   ```rust
   //! Request handlers split by feature cluster.
   //!
   //! Each submodule corresponds to a group of related `DaemonRequest` variants.
   //! The parent `handler` module re-exports everything so external import paths
   //! are unchanged.

   mod common;
   mod exec;
   mod image;
   mod lifecycle;
   mod logs;
   mod manifest;
   mod pipeline;
   mod run;
   mod snapshot;
   mod update;

   pub use common::*;
   pub use exec::*;
   pub use image::*;
   pub use lifecycle::*;
   pub use logs::*;
   pub use manifest::*;
   pub use pipeline::*;
   pub use run::*;
   pub use snapshot::*;
   pub use update::*;
   ```

2. Create `crates/minibox/src/daemon/handlers/common.rs` — extract from
   `handler.rs` lines 1–339 (imports, types, `PtySessionRegistry`,
   `NoopImageLoader`, `ImageDeps`, `LifecycleDeps`, `ExecDeps`, `BuildDeps`,
   `EventDeps`, `HandlerDependencies`, `ContainerPolicy`, `env_flag`,
   `validate_policy`, `send_error`, `generate_container_id`):

   Move verbatim. Update `use super::super::` to `use crate::daemon::` where needed.
   All types and functions keep their current visibility (`pub`, `pub(crate)`, or private).

3. Verify:

   ```
   cargo check -p minibox                   -> OK
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): create handlers/ module with common types (#356)"`

### Task 7: Extract run cluster to handlers/run.rs

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handlers/run.rs`
**Run**: `cargo nextest run -p minibox`

1. Move from `handler.rs` lines 340–1331 into `handlers/run.rs`:
   - `handle_run` (pub)
   - `handle_run_streaming` (private)
   - `prepare_run` (private)
   - `run_inner_capture` (private)
   - `run_inner` (private)
   - `run_from_params` (private)
   - `check_oom_killed` (private)
   - `daemon_wait_for_exit` (3 cfg variants)
   - `#[cfg(test)] mod run_inner_tests`

   Add `use super::common::*;` and any additional imports from the moved block.

2. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   cargo clippy -p minibox -- -D warnings    -> zero warnings
   ```

3. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): extract run cluster to handlers/run.rs (#356)"`

### Task 8: Extract lifecycle cluster to handlers/lifecycle.rs

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handlers/lifecycle.rs`
**Run**: `cargo nextest run -p minibox`

1. Move from `handler.rs` lines 1335–1657:
   - `handle_stop`, `stop_inner` (3 cfg variants)
   - `handle_pause`, `handle_resume`
   - `handle_remove`, `remove_inner`
   - `handle_list`

2. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   cargo clippy -p minibox -- -D warnings    -> zero warnings
   ```

3. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): extract lifecycle cluster to handlers/lifecycle.rs (#356)"`

### Task 9: Extract logs/events cluster to handlers/logs.rs

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handlers/logs.rs`
**Run**: `cargo nextest run -p minibox`

1. Move:
   - `handle_logs` (line 1667)
   - `handle_subscribe_events` (line 2455)

2. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   ```

3. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): extract logs/events to handlers/logs.rs (#356)"`

### Task 10: Extract image cluster to handlers/image.rs

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handlers/image.rs`
**Run**: `cargo nextest run -p minibox`

1. Move:
   - `resolve_platform_registry` (line 1763)
   - `handle_pull` (line 1793)
   - `handle_load_image` (line 1871)
   - `handle_push` (line 2134)
   - `handle_commit` (line 2232)
   - `handle_build` (line 2326)
   - `handle_prune` (line 2484)
   - `handle_remove_image` (line 2530)
   - `handle_list_images` (line 2585)
   - `#[cfg(test)] mod registry_router_tests`
   - `#[cfg(test)] mod pub_crate_handler_tests` (the image-related subset)

2. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   ```

3. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): extract image cluster to handlers/image.rs (#356)"`

### Task 11: Extract exec/PTY cluster to handlers/exec.rs

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handlers/exec.rs`
**Run**: `cargo nextest run -p minibox`

1. Move:
   - `handle_exec` (line 1935)
   - `handle_send_input` (line 2041)
   - `handle_resize_pty` (line 2091)

2. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   ```

3. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): extract exec/PTY to handlers/exec.rs (#356)"`

### Task 12: Extract remaining clusters (snapshot, pipeline, update, manifest)

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handlers/{snapshot,pipeline,update,manifest}.rs`
**Run**: `cargo nextest run -p minibox`

1. Move:
   - `handlers/snapshot.rs`: `handle_save_snapshot` (2606), `handle_restore_snapshot`
     (2636), `handle_list_snapshots` (2659)
   - `handlers/pipeline.rs`: `handle_pipeline` (2696)
   - `handlers/update.rs`: `handle_update` (2954)
   - `handlers/manifest.rs`: `handle_get_manifest` (3182), `handle_verify_manifest` (3266)

2. Verify:

   ```
   cargo nextest run -p minibox              -> all green
   cargo clippy -p minibox -- -D warnings    -> zero warnings
   ```

3. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): extract snapshot/pipeline/update/manifest handlers (#356)"`

### Task 13: Replace handler.rs with re-export shim

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handler.rs`, `crates/minibox/src/daemon/mod.rs`
**Run**: `cargo nextest run -p minibox`

1. Replace the entire contents of `handler.rs` with:

   ```rust
   //! Request handlers for each daemon operation.
   //!
   //! Implementation is split across submodules in `handlers/`. This file
   //! re-exports everything so that `use crate::daemon::handler::*` continues
   //! to work for all existing callers including server.rs and external tests.

   mod handlers;
   pub use handlers::*;
   ```

2. In `daemon/mod.rs`, no change needed — it already has `pub mod handler;`.

3. Verify:

   ```
   cargo nextest run --workspace            -> all green
   cargo clippy --workspace -- -D warnings  -> zero warnings
   cargo check --workspace                  -> OK
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "refactor(daemon): replace handler.rs with re-export shim, complete split (#356)"`

### Task 14: Final verification — full workspace gate

**Run**: `cargo xtask pre-commit`

1. Run the full pre-commit gate:

   ```
   cargo xtask pre-commit
   ```

2. Verify all external test files in `crates/minibox/tests/` still compile and pass.
   The import path `use minibox::daemon::handler::*` is preserved by the re-export.

3. Verify `server.rs` line 17 import still resolves:
   `use super::handler::{self, HandlerDependencies, handle_resize_pty, handle_send_input};`

4. Run: `git branch --show-current`
   Commit: `git commit -m "chore: verify full workspace gate after refactoring (#354 #355 #356)"`
