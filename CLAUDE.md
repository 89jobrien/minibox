# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Project Snapshot

Minibox is a Rust 2024 Docker-like container runtime with a daemon/CLI split, OCI image support, Linux namespace/cgroup isolation, overlay filesystems, and macOS adapter backends.

Default adapter selection lives in `miniboxd/src/adapter_registry.rs`: `smolvm` by default, falling back to `krun` when the `smolvm` binary is absent. Explicit `MINIBOX_ADAPTER=<value>` disables fallback.

## Read First

- `README.md` — user-facing overview and quickstart.
- `DEVELOPMENT.md` — canonical developer workflow and command selection.
- `docs/ARCHITECTURE.mbx.md` — workspace layout, crates, ports, adapter matrix, protocol overview.
- `docs/GOTCHAS.mbx.md` — non-obvious Rust/container/protocol pitfalls.
- `docs/TEST_INFRASTRUCTURE.mbx.md` — test categories, CI coverage, xtask commands.
- `docs/CRATE_INVENTORY.mbx.md` — crate/module inventory and current counts.
- `docs/FEATURE_MATRIX.mbx.md` — platform and adapter capability matrix.
- `docs/STATE_MODEL.mbx.md` — daemon persistence model.
- `docs/SECURITY_INVARIANTS.mbx.md` — security rules to preserve.

If changing container code, protocol types, adapters, or tests, read the relevant reference above instead of relying on this compact file.

## Environment Rules

- No vanilla Python: use `uv run` for Python scripts and `uv` for package management.
- Prefer Nushell or Rust for new scripts; if Python is necessary, use a uv script with inline metadata.
- No emojis in code or docs unless explicitly requested.
- Prefer editing existing files over creating new ones.
- Remove unused code completely; do not comment it out.
- Never commit secrets, credentials, or API keys.

## Core Commands

Use `just` or `cargo xtask` for repeatable gates.

- `cargo check --workspace` — compile/check workspace.
- `cargo xtask verify` — read-only local gate: fmt check, workspace check, clippy with warnings denied, borrow fixtures, docs lint.
- `cargo xtask borrow-fixtures` — standalone Rust borrow-reasoning must-pass/must-fail fixtures.
- `cargo xtask pre-commit` — macOS-safe pre-commit gate: fmt, clippy fixes/checks with warnings denied, release build.
- `cargo xtask prepush` — broader Linux-oriented gate: nextest and coverage.
- `cargo xtask test-unit` — cross-platform unit and conformance subset.
- `cargo xtask test-property` — property tests.
- `just test-integration` — Linux+root cgroup tests.
- `just test-e2e` — Linux+root daemon/CLI tests.
- `cargo xtask nuke-test-state` — clean orphaned containers, overlays, cgroups, and temp state.
- `cargo xtask build-vm-image` — build cached Alpine kernel/agent image for macOS VM adapters.
- `cargo xtask ci-watch [--branch <name>]` — watch latest GHA run with job-level detail; defaults
  to current branch. Nushell wrapper: `nu scripts/ci-watch.nu [--branch <name>]`.
- `cargo bench -p minibox` — local criterion benches.

`scripts/*.py` Claude Agent SDK scripts require an interactive foreground terminal and fail when run through background/non-interactive execution.

## Rust and Test Conventions

- Rust edition is 2024. `std::env::set_var` and `remove_var` are unsafe; serialize env-mutating tests with a shared lock.
- Treat warnings as errors in clippy runs: use `-D warnings` or the xtask/just gates.
- Gate Linux-only code and imports carefully. macOS `cargo check` does not validate `#[cfg(target_os = "linux")]` paths.
- Do not use `Command::cargo_bin()` for subprocess CLI tests; use the existing `find_minibox()`/`MINIBOX_TEST_BIN_DIR` pattern.
- Protocol changes start in `crates/minibox-core/src/protocol.rs`; update handlers, CLI paths, and snapshot tests together.
- New request fields should use `#[serde(default)]` when wire compatibility matters.
- Never discard handler channel-send failures with `let _ = ...`; log dropped-client cases.

## Architecture Guardrails

- Domain ports live in `minibox-core/src/domain.rs` and are implemented by adapters under `crates/minibox/src/adapters/`.
- `minibox` re-exports `minibox-core`; do not remove re-exports needed by `as_any!`/`adapt!` macro expansion.
- `DaemonRequest`/`DaemonResponse` are canonical in `crates/minibox-core/src/protocol.rs`.
- `DaemonResponse::ContainerOutput` is non-terminal; most other response variants end request streaming. Update terminal-response logic when adding variants.
- `HandlerDependencies` changes require updating all adapter suite construction sites in `miniboxd/src/main.rs`.

## Security Invariants

- Preserve tar extraction protections: reject `..`, absolute symlinks, device nodes, FIFOs, and setuid/setgid bits.
- Keep overlay/path validation inside the target root.
- Preserve Unix socket peer credential checks and root-only access.
- Enforce image pull size limits.
- Container init must use `execve` with explicit env, not `execvp`.

## Git Workflow

Branches follow the stability pipeline:

`develop` -> `next` -> `staging` -> `main` -> `v*` tag

- Target feature, hotfix, and chore work at `develop`.
- Do not promote `next` to `staging` without confirming `next` CI is green.
- Do not promote `staging` to `main` without confirming `staging` CI is green.
- Do not commit unless explicitly asked.
- `.ctx/HANDOFF.*.*.yaml` is gitignored by default; use `git add -f` only when intentionally tracking it.

## Hook Notes

Claude hook config lives in `.claude/settings.json`. The `SessionStart` hook runs `nu scripts/preflight.nu`; it should be fast, read-only, and non-fatal so startup is not blocked by normal local state like an uncommitted working tree.

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
