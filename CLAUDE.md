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






<!-- cloude-code-toolbox:mcp-skills-awareness-begin -->

### MCP & Skills awareness (Cloude Code ToolBox)

_Last synced: 2026-05-18T16:21:06.033093Z._

- **Full report:** `.claude/cloude-code-toolbox-mcp-skills-awareness.md`

#### Workspace MCP

- `/Users/joe/dev/minibox/.vscode/mcp.json` — _servers defined_

- **github** (stdio)
- **personal** (stdio)

#### User MCP

- `/Users/joe/Library/Application Support/Code/User/mcp.json` — _servers defined_

- **personal** (stdio)

<!-- cloude-code-toolbox:mcp-skills-awareness-end -->
<!-- cloude-code-memory-bank:begin -->
# Plan / Act workflow (Cursor-style)

Unless the user clearly opts out (e.g. **"skip plan, implement now"** or **"just fix it"** with no ambiguity), use **two modes**. This matches Cursor’s PLAN → approve → ACT flow.

## Plan mode (default)

- **First line of every Plan-mode response MUST be exactly:** `# Mode: PLAN`
- **Do not modify the repository in any way**, including:
  - No creating, editing, or deleting files (source, config, docs, **including `./memory-bank/**` memory-bank files**).
  - No applying multi-file edits, quick fixes, or patch-style changes.
  - No terminal commands that change the workspace (installs, builds that write outputs you were asked to apply, `git` writes, etc.).
- **Allowed in Plan mode:** Read/search files to understand the codebase, answer questions, list steps, identify risks, and produce a **written plan** (markdown).
- **End Plan-mode responses** by telling the user how to proceed, e.g. **Type `ACT` when you approve this plan** (or ask them to refine the plan first).

## Act mode

- Enter **only** when the user’s message **clearly approves implementation**, e.g. they send **`ACT`**, **`act`**, or phrases like **"go ahead"**, **"implement the plan"**, **"approved"** right after a plan—or they explicitly told you to skip planning and implement.
- **First line of every Act-mode response MUST be exactly:** `# Mode: ACT`
- **Then** you may edit files, run commands, and update **`./memory-bank/`** when appropriate.
- After you finish an Act-mode turn, assume the next user message starts in **Plan mode** again unless they again approve with **`ACT`** (or equivalent) for further edits.

## If the user asks for code changes while you are in Plan mode

- **Do not implement.** Respond with `# Mode: PLAN`, briefly restate or adjust the plan, and ask them to type **`ACT`** when they want you to apply changes.

---

# Memory bank (persistent context)

This repository uses a **memory bank** under `./memory-bank/` — structured markdown that survives sessions, similar to Cursor-style workflows.

Context layers (read deeper files after foundations): **projectbrief** → **productContext** / **systemPatterns** / **techContext** → **activeContext** → **progress**.

## What Claude should do

1. **Before substantive work**, read **all** of the following under `./memory-bank/` when the task depends on project state (not optional for non-trivial work). In **Plan mode**, reading for the plan is allowed; **do not edit** these files until **Act mode** unless the user only asked for a documentation/memory update with no code change.
   - `projectbrief.md` — scope and goals
   - `productContext.md` — product intent and UX
   - `systemPatterns.md` — architecture and conventions
   - `techContext.md` — stack and constraints
   - `progress.md` — done / pending / known issues
   - `activeContext.md` — current task and decisions

2. **During Act-mode work**, keep `activeContext.md` aligned with the current task (update when focus shifts).

3. **After meaningful milestones** (in Act mode), update `progress.md` and any affected docs in `./memory-bank/`.

4. When the user asks to **update memory bank** (or similar), **open and review every** file in `./memory-bank/`, then update what changed — especially `activeContext.md` and `progress.md`, even if other files are unchanged. Prefer doing heavy memory-bank writes in **Act mode** unless the user asked for documentation-only updates.

5. Prefer **short, factual updates** over long prose. Reference files, symbols, and tickets instead of duplicating code.

Do not delete these files; evolve them as the project changes.
<!-- cloude-code-memory-bank:end -->