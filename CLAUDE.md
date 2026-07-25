# CLAUDE.md

@.claude.local.md

Guidance for Claude Code when working in this repository.

## Project Snapshot

Minibox is a Rust 2024 Docker-like container runtime with a daemon/CLI split, OCI image support, Linux namespace/cgroup isolation, overlay filesystems, and macOS adapter backends.

Default adapter selection lives in `crates/miniboxd/src/adapter_registry.rs`: `smolvm` by default, falling back to `native` on Linux or `krun` on macOS when the `smolvm` binary is absent. Explicit `MINIBOX_ADAPTER=<value>` disables fallback.

## Read First

- `README.md` — user-facing overview and quickstart.
- `DEVELOPMENT.md` — canonical developer workflow and command selection.
- `docs/core/ARCHITECTURE.mbx.md` — workspace layout, crates, ports, adapter matrix, protocol overview.
- `docs/core/GOTCHAS.mbx.md` — non-obvious Rust/container/protocol pitfalls.
- `docs/core/TEST_INFRASTRUCTURE.mbx.md` — test categories, CI coverage, xtask commands.
- `docs/core/CRATE_INVENTORY.mbx.md` — crate/module inventory and current counts.
- `docs/core/FEATURE_MATRIX.mbx.md` — platform and adapter capability matrix.
- `docs/core/STATE_MODEL.mbx.md` — daemon persistence model.
- `docs/core/SECURITY_INVARIANTS.mbx.md` — security rules to preserve.

If changing container code, protocol types, adapters, or tests, read the relevant reference above instead of relying on this compact file.

## Environment Rules

- No vanilla Python: use `uv run` for Python scripts and `uv` for package management.
- Prefer Nushell or Rust for new scripts; if Python is necessary, use a uv script with inline metadata.
- No emojis in code or docs unless explicitly requested.
- Prefer editing existing files over creating new ones.
- Remove unused code completely; do not comment it out.
- Never commit secrets, credentials, or API keys.
- Fuzz seeds, corpus, and artifacts are gitignored via `**/fuzz/seeds/`,
  `**/fuzz/corpus/`, `**/fuzz/artifacts/`. Do not commit generated fuzz data.

## Test Daemon Gotchas

- Never use `Stdio::piped()` for daemon stdout/stderr in tests unless you drain
  the pipes in a background thread. Debug logging during slow operations (pull,
  run) can exceed the 64KB OS pipe buffer and deadlock the daemon. Use
  `Stdio::null()` or reduce log level to `warn`/`info`.
- `cargo fmt --all --check` (run by pre-commit) formats ALL workspace files
  including untracked ones. Run `cargo fmt --all` before committing if untracked
  `.rs` files exist.

## Core Commands

Use `just` or `cargo xtask` for repeatable gates.

- `cargo check --workspace` — compile/check workspace.
- `cargo xtask verify` — read-only local gate: fmt check, workspace check, clippy with warnings denied, borrow fixtures, docs lint.
- `cargo xtask borrow-fixtures` — standalone Rust borrow-reasoning must-pass/must-fail fixtures.
- `cargo xtask pre-commit` — macOS-safe pre-commit gate: fmt, clippy fixes/checks with warnings denied, release build.
- `cargo xtask prepush` — broader Linux-oriented gate: nextest (use `cargo xtask coverage` separately for coverage reports).
- `cargo xtask test-unit` — cross-platform unit and conformance subset.
- `cargo xtask test-property` — property tests.
- `just test-integration` — Linux+root cgroup tests.
- `just test-e2e` — Linux+root daemon/CLI tests.
- `cargo xtask nuke-test-state` — clean orphaned containers, overlays, cgroups, and temp state.
- `cargo xtask build-vm-image` — build cached Alpine kernel/agent image for macOS VM adapters.
- `cargo xtask ci-watch [--branch <name>]` — watch latest GHA run with job-level detail; defaults
  to current branch. Nushell wrapper: `nu scripts/ci-watch.nu [--branch <name>]`.
- `cargo xtask bench` — run criterion benchmarks and save results to `bench/results/`.

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

- Domain ports live in `crates/minibox/src/domain.rs` and are implemented by adapters under `crates/minibox/src/adapters/`.
- `minibox` re-exports `minibox-core`; do not remove re-exports needed by `as_any!`/`adapt!` macro expansion.
- `DaemonRequest`/`DaemonResponse` are canonical in `crates/minibox-core/src/protocol.rs`.
- `DaemonResponse::ContainerOutput` is non-terminal; most other response variants end request streaming. Update terminal-response logic when adding variants.
- `HandlerDependencies` changes require updating all adapter suite construction sites in `crates/miniboxd/src/main.rs`.

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

### Branch Protection

- `main` is protected via GitHub rulesets. All changes land via PR
  with required status checks. Branch must be up-to-date before merge.
- `next` and `staging` block force pushes and deletions; require
  `CI passed` status check.
- `staging` -> `main` promotion creates a PR automatically via CI.
- Required status checks on `main`: `CI passed`, stability gates,
  `actionlint`.

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
New adapter? Update composition   → crates/miniboxd/src/main.rs (all suites)
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
<!-- godmode-workflow:begin -->

# Phased workflow

Unless the user clearly opts out (e.g. **"skip plan, just fix it"**), every
non-trivial task progresses through five phases. Short confirmations like
**"do it"**, **"act"**, **"go"** advance to the next phase.

## Phases

<godmode-phase name="ORIENT" mode="read-only" response-header="# Phase: ORIENT" skills="godmode handon">
Default phase. Read files, search code, run `godmode handon`, check task
graph. Summarize current state: branch, dirty files, relevant context.
No modifications to the repository. End by stating what you found and
what phase comes next.
</godmode-phase>

<godmode-phase name="PLAN" mode="read-only" response-header="# Phase: PLAN" skills="brainstorm, writing-plans">
Produce a written plan: files to touch, approach, risks. Still read-only
— no edits, no builds that write output. For complex work, invoke
`godmode:brainstorm` or `godmode:writing-plans`. End with "Type ACT to
proceed" (or suggest refinements).
</godmode-phase>

<godmode-phase name="ACT" mode="read-write" response-header="# Phase: ACT" skills="task-driven-development, parallel-agents">
Enter when the user approves: "act", "go ahead", "do it". Edit files,
run commands, dispatch subagents. For multi-task work, use
`godmode:task-management` and `godmode:parallel-agents` when tasks are
independent. After finishing, transition to VERIFY automatically.
</godmode-phase>

<godmode-phase name="VERIFY" mode="read + test" response-header="# Phase: VERIFY" skills="verification-before-completion">
Run `cargo check`, `cargo clippy`, `cargo test` (or `godmode verify`).
Invoke `godmode:verification-before-completion` for non-trivial changes.
Report results. If failures exist, return to ACT to fix them. When
green, state readiness and ask to SHIP.
</godmode-phase>

<godmode-phase name="SHIP" mode="commit/push" response-header="# Phase: SHIP" skills="cap, handoff">
Commit, push, update handoff: `godmode:cap`, then `godmode handoff`.
Only entered with explicit user approval. After shipping, return to
ORIENT for the next task.
</godmode-phase>

## Phase transitions

- **User can skip phases**: "skip plan, implement now" jumps to ACT.
  "just fix it" implies ORIENT → ACT → VERIFY → SHIP in one pass.
- **After each ACT turn**, default back to VERIFY unless the user says
  otherwise.
- **Multiple ACT turns** are fine — the user can keep approving.
- When the user gives a lettered choice or short confirmation, advance
  to the most obvious next phase without asking.

## Skill invocation rule

Before responding in any phase, check if a godmode skill applies.
1% chance it’s relevant = invoke it. Process skills (`brainstorm`,
`systematic-debugging`) before implementation skills
(`task-driven-development`, `parallel-agents`).

## Task graph

Tasks live in @.ctx/GODMODE.tasks.yaml. Use `Bash(godmode task)` CLI for
state transitions. Independent chains can run in parallel via
`Skill(godmode:parallel-agents)`. A task is runnable when all `depends_on`
items are `done`.

## Memory bank

- Persistent context lives in @.ctx/memory-bank/
- Read before substantive work: !`ls .ctx/memory-bank/`
- update after milestones: @.ctx/memory-bank/activeContext.mbx.md and @.ctx/memory-bank/progress.mbx.md
- See @AGENTS.md for the full file list

## Context Graph

- Wiki root: `Read(.kgx/wiki/index.md)`
- Query the graph: !`kgx query <entity>`
-

## Agent-specific guidance

For subagent conventions, Codex integration, and memory-bank file
inventory, see @AGENTS.md.

<!-- godmode-workflow:end -->
