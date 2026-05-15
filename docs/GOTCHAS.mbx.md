# Gotchas and Non-Obvious Patterns

Last updated: 2026-05-08

Deep reference for debugging container init, cgroups, proptest, macros, and protocol edges.
For Rust coding conventions see `.claude/rules/rust-patterns.md`.

## Testing

### General

- **`std::env::set_var`/`remove_var` are `unsafe` in Rust 2024** — wrap in `unsafe {}` and
  serialise with a `static Mutex<()>` guard to prevent parallel test races (see
  `commands/mod.rs` tests for the pattern).
- **Crate extraction + dev-deps**: When moving types to a new crate, check `tests/` files in
  the source crate — they may directly use crates that were previously transitive (e.g. `sha2`).
  Add them explicitly to `[dev-dependencies]` or `cargo nextest` will catch it at push time.
- **Subprocess tests**: Use `Command::from_std(std::process::Command::new(find_minibox()))` +
  `MINIBOX_TEST_BIN_DIR` env var — never `Command::cargo_bin()`, which triggers a full
  recompile. Gate with `#![cfg(all(unix, feature = "subprocess-tests"))]`; run via
  `just test-cli-subprocess`.

### Proptest

- **`FileFailurePersistence` warning** — proptest can't find `lib.rs` from the `tests/`
  context; suppress with `ProptestConfig { failure_persistence: None, .. }` inside each
  `proptest!` block.
- **Async methods in proptest** — use `tokio::runtime::Runtime::new().unwrap().block_on(...)`
  to drive async calls synchronously inside proptest closures. Each closure must create its
  own `Runtime`.
- **`DaemonState` fixture** — requires `ImageStore::new(tmp.join("images"))` + a `data_dir`
  path; calls `save_to_disk` on every mutation (~256 JSON writes per proptest run). Use a named
  `TempDir` so it stays alive for the closure.
- **`CgroupManager::create()` runs `create_dir_all` before bounds checks** — proptest cgroup
  bound tests need a real cgroup2 mount and root. Gate with `#[cfg(target_os = "linux")]` and
  run under `just test-integration`.

## Macros and Doctests

- **`as_any!` macro uses `crate::domain::AsAny`** — `crate` resolves at the call site
  (minibox), not the defining crate (minibox-macros). Suppress `crate_in_macro_def` with
  `#[allow(clippy::crate_in_macro_def)]`; do not change to `$crate`.
- **`adapt!` requires `new() -> Self`** — `adapt!` calls `default_new!` which implements
  `Default` via `Self::new()`. Adapters whose `new()` returns `Result<Self>` must use
  `as_any!` only.
- **Private fn doctests** — mark with ` ```ignore ` (not `no_run`); private functions are
  inaccessible in doctest context and will fail to compile.

## Protocol (`protocol.rs` / `handler.rs`)

- **Single `DaemonRequest` definition** — canonical source is
  `crates/minibox-core/src/protocol.rs`. `minibox` re-exports it. Wire format snapshot tests
  pin serialization; add a snapshot test when adding a field.
- **`HandlerDependencies` construction sites** — Adding fields requires updating all five
  adapter suites in `miniboxd/src/main.rs` (native, gke, colima, smolvm, krun). These are
  `#[cfg(target_os = "linux")]` and won't fail on macOS `cargo check`.
- **`handle_run` param chain** — Adding a parameter requires updating in order:
  `daemon/server.rs` dispatch → `handle_run` → `handle_run_streaming` → `run_inner_capture` →
  `run_inner`. All five sites must change together.
- **`#[serde(default)]` for backward-compatible additions** — New fields on `DaemonRequest`
  variants must use `#[serde(default)]` so existing clients that omit the field continue to
  work.
- **Silent channel-send discards are a bug** — never `let _ = tx.send(...).await`. Use
  `if tx.send(...).await.is_err() { warn!(...) }` so dropped connections are observable.
- **Stale rust-analyzer diagnostics** — use `cargo check -p <crate>` as source of truth, not
  the IDE error count.
- **Linux-only clippy lints** — `#![cfg(target_os = "linux")]` files are invisible to macOS
  clippy. Lints like `clone_on_copy` only surface on Linux CI runners.
- **Linux-only cfg-gated imports** — manually verify all `use` statements in Linux-only modules
  (`cgroup_tests.rs`, `bridge.rs`, etc.) — the macOS compiler won't catch missing imports.

## macbox

- **Stale crate name** — the lib crate was briefly `linuxbox` (2026-04-21 to 2026-04-26). Any
  `linuxbox::` reference is stale; use `minibox::`.
- **App Sandbox blocks fork** — see "macOS Notarization / App Sandbox Constraints" in CLAUDE.md
  for the full SBPL allowlist.

## Container Init (`filesystem.rs` / `process.rs`)

- **Pipe fds across `clone()`** — both parent and child get copies of any `OwnedFd` after
  clone. Use `std::mem::forget` on fds before the clone call, then manage raw fds manually.
  Child: `dup2` write end into stdout/stderr, then close both raw fds. Parent: drop write end
  after clone, keep read end for output streaming.
- **`pivot_root` requires `MS_PRIVATE` first** — after `CLONE_NEWNS` the child inherits shared
  mount propagation; `pivot_root` fails EINVAL unless you call
  `mount("", "/", MS_REC|MS_PRIVATE)` before the bind-mount.
- **`close_extra_fds` uses `close_range(2)` fast path** — tries syscall first (kernel 5.9+).
  Falls back to `/proc/self/fd` iteration which must collect FD numbers into a `Vec` before
  closing (iterating and closing in the loop would close `ReadDir`'s own FD).
- **Absolute symlink rewrite in `layer.rs`** — `strip_prefix("/")` gives a path relative to
  the container root, not the symlink's directory. Use `relative_path(entry_dir, abs_target)`
  to get the correct relative target; otherwise busybox applet symlinks break.
- **Tar root entries** — `"."` and `"./"` entries must be skipped before path validation;
  `Path::join("./")` normalizes away the CurDir component, causing a false path-escape error.
- **`child_init` uses `execve` not `execvp`** — `execvp` inherits the daemon's host env.
  `child_init` calls `execve` with an explicit `envp` from `config.env`. Do not revert to
  `execvp`.

## Cgroup v2 (`cgroups.rs`)

- `io.max` requires `MAJOR:MINOR` of a real block device — Colima VM uses virtio (`vda` =
  253:0). Use `find_first_block_device()` (reads `/sys/block/*/dev`) rather than hardcoding.
- PID 0 is silently accepted by kernel 6.8 but is never valid — validate before writing to
  `cgroup.procs`.
- A cgroup cannot have both processes AND children (v2 "no internal process" rule). Tests run
  inside `minibox-test-slice/runner-leaf` via `cargo xtask run-cgroup-tests`.
