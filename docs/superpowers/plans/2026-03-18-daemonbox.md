---
status: done
completed: "2026-03-18"
branch: main
note: daemonbox extracted and live
---
# daemonbox — Extract Shared Daemon Logic Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `handler.rs`, `state.rs`, and `server.rs` out of `miniboxd` into a new shared `daemonbox` crate so both `miniboxd` (Linux) and `macboxd` (macOS, future) can depend on them without duplication.

**Architecture:** `daemonbox` is a new library crate that receives the three files verbatim — the intra-crate `use crate::...` references in those files all resolve correctly in the new home. `miniboxd/src/lib.rs` is replaced with a one-liner re-export shim (`pub use daemonbox::*`) so the twelve existing integration tests that import `miniboxd::handler` and `miniboxd::state` continue to compile without any changes. No handler logic changes, no protocol changes, no new behaviour.

**Platform note:** `daemonbox` compiles on Linux and macOS (both are unix, both have `tokio::net::UnixStream`). It does not need to compile on Windows — `winboxd` is a Named Pipe proxy that does not embed `daemonbox`.

**Tech Stack:** Rust workspace, `minibox-lib` (domain traits), `tokio`, `nix`, `serde`/`serde_json`, `uuid`, `chrono`, `anyhow`, `tracing`.

**Spec:** `docs/superpowers/specs/2026-03-17-macbox-daemonbox-design.md`

**Important:** Use per-crate `-p` flags throughout (`cargo check -p daemonbox -p miniboxd`). Do not use `--workspace` — `miniboxd` has `compile_error!` on macOS and `macboxd` will have `compile_error!` on Linux, so workspace-wide commands always fail on one platform.

---

## File Map

### New files

| File                              | Responsibility                                                              |
| --------------------------------- | --------------------------------------------------------------------------- |
| `crates/daemonbox/Cargo.toml`     | Crate manifest                                                              |
| `crates/daemonbox/src/lib.rs`     | `pub mod handler; pub mod server; pub mod state;`                           |
| `crates/daemonbox/src/handler.rs` | Copied verbatim from `miniboxd/src/handler.rs` — request handlers, `HandlerDependencies` |
| `crates/daemonbox/src/state.rs`   | Copied verbatim from `miniboxd/src/state.rs` — `DaemonState`, `ContainerRecord` |
| `crates/daemonbox/src/server.rs`  | Copied verbatim from `miniboxd/src/server.rs` — Unix socket connection handler |

### Modified files

| File                          | Change                                                                              |
| ----------------------------- | ----------------------------------------------------------------------------------- |
| `Cargo.toml`                  | Add `daemonbox` to `[workspace]` members and `[workspace.dependencies]`            |
| `crates/miniboxd/Cargo.toml`  | Add `daemonbox = { workspace = true }` to `[dependencies]`                         |
| `crates/miniboxd/src/lib.rs`  | Replace module declarations with `pub use daemonbox::{handler, server, state};`    |

### Deleted files

| File                              | When                                       |
| --------------------------------- | ------------------------------------------ |
| `crates/miniboxd/src/handler.rs`  | After verified compile with shim (Task 3)  |
| `crates/miniboxd/src/state.rs`    | After verified compile with shim (Task 3)  |
| `crates/miniboxd/src/server.rs`   | After verified compile with shim (Task 3)  |

---

## Task 1: Scaffold the `daemonbox` crate

**Files:**
- Create: `crates/daemonbox/Cargo.toml`
- Create: `crates/daemonbox/src/lib.rs` (stub)
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add `daemonbox` to workspace**

In root `Cargo.toml`, add to the `[workspace]` members list:

```toml
"crates/daemonbox",
```

Add to `[workspace.dependencies]`:

```toml
daemonbox = { path = "crates/daemonbox" }
```

- [ ] **Step 2: Create directory structure**

```bash
mkdir -p crates/daemonbox/src
```

- [ ] **Step 3: Create `crates/daemonbox/Cargo.toml`**

The dependency list mirrors `miniboxd/Cargo.toml` minus `tracing-subscriber` (that's a binary concern, not a library concern):

```toml
[package]
name = "daemonbox"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
minibox-lib = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
nix = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 4: Create stub `crates/daemonbox/src/lib.rs`**

```rust
//! Shared daemon application layer.
//!
//! Contains the request handlers, in-memory state, and Unix socket server
//! used by both `miniboxd` (Linux) and `macboxd` (macOS).

pub mod handler;
pub mod server;
pub mod state;
```

- [ ] **Step 5: Create empty placeholder files so the crate compiles**

Create `crates/daemonbox/src/handler.rs`, `crates/daemonbox/src/state.rs`, and `crates/daemonbox/src/server.rs` each containing only a comment:

```rust
// placeholder — content copied in Task 2
```

- [ ] **Step 6: Verify the scaffold compiles**

```bash
cargo check -p daemonbox
```

Expected: `Finished` with no errors. If it fails, check that all three placeholder files exist and that the workspace `Cargo.toml` edit is correct.

- [ ] **Step 7: Commit the scaffold**

```bash
git add crates/daemonbox/ Cargo.toml
git commit -m "chore: scaffold daemonbox crate (empty placeholders)"
```

---

## Task 2: Copy source files into `daemonbox`

**Files:**
- Modify: `crates/daemonbox/src/handler.rs` (replace placeholder with content from `miniboxd`)
- Modify: `crates/daemonbox/src/state.rs` (replace placeholder with content from `miniboxd`)
- Modify: `crates/daemonbox/src/server.rs` (replace placeholder with content from `miniboxd`)

The three files contain only `use crate::...` and `use minibox_lib::...` references — no `use miniboxd::` references — so they can be copied verbatim and will compile immediately in the new crate.

- [ ] **Step 1: Copy `handler.rs`**

Copy `crates/miniboxd/src/handler.rs` to `crates/daemonbox/src/handler.rs`, replacing the placeholder entirely. Do not modify any content.

- [ ] **Step 2: Copy `state.rs`**

Copy `crates/miniboxd/src/state.rs` to `crates/daemonbox/src/state.rs`, replacing the placeholder entirely. Do not modify any content.

- [ ] **Step 3: Copy `server.rs`**

Copy `crates/miniboxd/src/server.rs` to `crates/daemonbox/src/server.rs`, replacing the placeholder entirely. Do not modify any content.

- [ ] **Step 4: Verify `daemonbox` compiles with the real source**

```bash
cargo check -p daemonbox
```

Expected: `Finished` with no errors.

If there are import errors, the most likely cause is a missing dependency in `daemonbox/Cargo.toml`. Check the `use` statements at the top of each file against the Cargo.toml deps list. Do not add `libc` — it is used transitively via `nix`, not directly.

- [ ] **Step 5: Run clippy on `daemonbox`**

```bash
cargo clippy -p daemonbox -- -D warnings
```

Expected: no warnings. Do not suppress warnings — fix them.

- [ ] **Step 6: Run fmt check**

```bash
cargo fmt -p daemonbox --check
```

Expected: no diff. If there is a diff, run `cargo fmt -p daemonbox` and stage the changes.

- [ ] **Step 7: Commit**

```bash
git add crates/daemonbox/src/
git commit -m "feat: populate daemonbox with handler/state/server (copies from miniboxd)"
```

---

## Task 3: Wire `miniboxd` to `daemonbox` and delete originals

**Files:**
- Modify: `crates/miniboxd/Cargo.toml`
- Modify: `crates/miniboxd/src/lib.rs`
- Delete: `crates/miniboxd/src/handler.rs`
- Delete: `crates/miniboxd/src/state.rs`
- Delete: `crates/miniboxd/src/server.rs`

- [ ] **Step 1: Add `daemonbox` dependency to `miniboxd`**

In `crates/miniboxd/Cargo.toml`, add to `[dependencies]`:

```toml
daemonbox = { workspace = true }
```

- [ ] **Step 2: Replace `miniboxd/src/lib.rs` with re-export shim**

Replace the entire content of `crates/miniboxd/src/lib.rs` with:

```rust
//! miniboxd library — re-exports from daemonbox for backward compatibility.
//!
//! These re-exports exist so that integration tests importing
//! `miniboxd::handler`, `miniboxd::state`, or `miniboxd::server` continue
//! to compile without changes after the move to `daemonbox`.

#[doc(hidden)]
pub use daemonbox::handler;
#[doc(hidden)]
pub use daemonbox::server;
#[doc(hidden)]
pub use daemonbox::state;
```

- [ ] **Step 3: Verify both crates compile with the shim in place (originals still exist)**

```bash
cargo check -p daemonbox -p miniboxd
```

Expected: `Finished` with no errors. Do not proceed to deletion until this passes — the originals are still present, so `miniboxd` currently has the real modules AND the re-exports. That's fine; the compiler will use the re-exports from `lib.rs`.

- [ ] **Step 4: Delete the original source files from `miniboxd`**

```bash
rm crates/miniboxd/src/handler.rs
rm crates/miniboxd/src/state.rs
rm crates/miniboxd/src/server.rs
```

- [ ] **Step 5: Verify both crates still compile after deletion**

```bash
cargo check -p daemonbox -p miniboxd
```

Expected: `Finished` with no errors. If `miniboxd` now errors, check that `main.rs` imports resolve — `main.rs` uses `miniboxd::handler::HandlerDependencies` and `miniboxd::state::DaemonState`, which both flow through the shim.

- [ ] **Step 6: Run the existing handler tests**

These tests are in `crates/miniboxd/tests/handler_tests.rs` and import `miniboxd::handler` and `miniboxd::state`. They must pass unchanged — this is the correctness gate for the refactor.

```bash
cargo nextest run -p miniboxd
```

Expected: all tests pass (12 handler tests + any other miniboxd tests). If any test fails, do not proceed — diagnose and fix before continuing.

- [ ] **Step 7: Run tests on `daemonbox` itself**

```bash
cargo nextest run -p daemonbox
```

Expected: all tests pass (daemonbox has no tests of its own yet — this just confirms the test harness runs cleanly).

- [ ] **Step 8: Final clippy pass on both crates**

```bash
cargo clippy -p daemonbox -p miniboxd -- -D warnings
```

Expected: no warnings.

- [ ] **Step 9: Remove unused `libc` dep from `miniboxd/Cargo.toml`**

After moving the three files, `miniboxd`'s own sources (`main.rs`, `lib.rs`) no longer use `libc` directly — `daemonbox` now owns that transitive need via `nix`. Remove it:

In `crates/miniboxd/Cargo.toml`, delete the line:

```toml
libc = { workspace = true }
```

Then verify:

```bash
cargo check -p miniboxd
```

Expected: `Finished` with no errors.

- [ ] **Step 10: Commit**

```bash
git add crates/miniboxd/src/lib.rs crates/miniboxd/Cargo.toml
git rm crates/miniboxd/src/handler.rs crates/miniboxd/src/state.rs crates/miniboxd/src/server.rs
git commit -m "refactor: extract handler/state/server into daemonbox crate"
```

---

## Verification Summary

| Check                        | Command                                           | Platform     | Expected                       |
| ---------------------------- | ------------------------------------------------- | ------------ | ------------------------------ |
| daemonbox compiles           | `cargo check -p daemonbox`                        | Linux/macOS  | No errors                      |
| miniboxd compiles            | `cargo check -p miniboxd`                         | Linux        | No errors                      |
| miniboxd tests pass          | `cargo nextest run -p miniboxd`                   | Linux        | All pass (12 handler tests etc) |
| minibox-lib unaffected       | `cargo check -p minibox-lib`                      | any          | No errors                      |
| clippy clean                 | `cargo clippy -p daemonbox -p miniboxd -D warnings` | Linux      | No warnings                    |
| fmt clean                    | `cargo fmt -p daemonbox --check`                  | any          | No diff                        |
