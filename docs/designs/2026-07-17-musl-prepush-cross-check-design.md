# Design: musl release cross-check in prepush gate

## Goal

Catch `x86_64-unknown-linux-musl` release-build breakage (`cfg(target_os = "linux")`
divergence) in the `prepush` xtask gate, before code reaches CI — preventing repeats of
the 2026-05-22 incident (5 iterative CI-fix-push cycles; see
`.ctx/memory-bank/mistakes.md`, "xtask: path/binary resolution errors").

## Approved Approach

Standalone `cargo zigbuild` release cross-build shelled out directly from a new helper
in `xtask/src/gates.rs`, independent of the existing `test_linux::Compiler` trait (that
trait is scoped to debug builds for the VM-test pipeline; extending it for a release
flag would force unrelated callers — `test_in_vm.rs`, `test_image.rs` — to thread a
parameter they don't need).

## Context Map

### Files to Modify

| File | Purpose | Changes Needed |
| --- | --- | --- |
| `xtask/src/gates.rs` | Gate definitions | Replace stale TODO (lines 187-189) with `cross_check_musl_release()` helper + local `MuslCrossCheckError` type; call helper at the top of `prepush()` |
| `xtask/Cargo.toml` | xtask deps | Add `miette = { workspace = true }`, `thiserror = { workspace = true }` |

### Dependencies (may need updates)

None. `prepush()` is called only from `xtask/src/main.rs:86` (`Some("prepush") => gates::prepush(&sh)`), which stays `anyhow::Result<()>` — no signature change, so no consumer updates needed.

### Test Coverage

| Test | Covers |
| --- | --- |
| *(none found)* | `prepush()` has no existing test coverage — it's a shell-out integration gate. This gap is **pre-existing** and out of scope for this change; not introduced by it. |

### Reference Patterns

| File | Pattern to Follow |
| --- | --- |
| `xtask/src/test_linux.rs:59-66` (`run_cargo_zigbuild`) | Shelling out to `cargo zigbuild` via `std::process::Command`, checking `status.success()` |
| `crates/minibox-core/src/error.rs` | Model for `#[derive(thiserror::Error, miette::Diagnostic)]` with `code(...)` + `help(...)` |
| `xtask/src/gates.rs` (`lint`, other gates) | `gated(GateId::X, &root, move || { ... })` closure pattern, `cmd!(sh, "...")` for shell commands |

### Risk

- [x] Public API change: **no** — `prepush()` signature unchanged, no `pub` types modified
- [x] New external dependency: **yes** — `miette` + `thiserror` added to `xtask/Cargo.toml`. Both already exist as workspace-pinned versions (`miette = "7"` with `fancy`, `thiserror = "2"`) used elsewhere in the workspace, so this is a version-consistent addition, not a new pin. **Deliberately out of the documented miette rollout scope** (`docs/plans/2026-07-07-structured-errors-miette.md` lists `xtask` as unaffected) — approved as a scoped, intentional carve-out for this one error path.
- [x] Feature flag required: **no**

## Crate Ownership

- **Owner crate**: `xtask` — build/CI tooling lives here exclusively; this is a gate addition, not product code.
- **Affected crates**: none downstream (xtask is a leaf binary crate, not depended on by anything else in the workspace).

## Public API

No new `pub` items. Everything below is private to `xtask/src/gates.rs`.

### Types

```rust
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("x86_64-unknown-linux-musl release cross-build failed")]
#[diagnostic(
    code(xtask::musl_cross_check),
    help("run `rustup target add x86_64-unknown-linux-musl`, or install cargo-zigbuild if missing")
)]
struct MuslCrossCheckError;
```

### Functions

```rust
/// Cross-compile `miniboxd` + `mbx` in release mode for the VPS deploy target
/// (`x86_64-unknown-linux-musl`) via `cargo zigbuild`. Hard-fails with a
/// rendered miette diagnostic on any failure (missing target, missing
/// zigbuild, or actual compile/link error).
fn cross_check_musl_release(sh: &Shell) -> Result<()>;
```

Called once, at the top of the existing `gated(GateId::Prepush, ...)` closure in
`prepush()`, before the current native `cargo build --release ...` step.

## Data Flow

1. **Source**: `prepush()` gate invocation (`cargo xtask prepush`, or pre-push git hook).
2. **Transform**: `cross_check_musl_release(sh)` shells out to
   `cargo zigbuild --release -p miniboxd -p mbx --target x86_64-unknown-linux-musl`
   via `xshell::cmd!` (consistent with the rest of `gates.rs`), checking exit status.
3. **Sink (success)**: falls through to the existing native release build + nextest + conformance steps, unchanged.
4. **Sink (failure)**: constructs `MuslCrossCheckError`, renders it via
   `eprintln!("{:?}", miette::Report::new(MuslCrossCheckError))` for fancy colored
   output (code + help text), then converts to `anyhow::Error` (e.g.
   `.context("musl cross-check failed")` on the `cmd!` result, or `bail!(...)` after
   the manual render) to satisfy `prepush()`'s existing `anyhow::Result<()>` chain.

No fallback to `musl-gcc`/`CC_<target>` env vars (unlike `test_linux::ZigbuildCompiler`)
— `cargo-zigbuild` is confirmed installed on the dev machine, and this is a local
dev-machine gate, not a CI job running on an unconfigured environment.

## Hexagonal Boundaries

Not applicable in the traditional port/adapter sense — this is a single shell-out
helper in build tooling, not domain logic. No trait introduced; deliberately not
routed through `test_linux::Compiler` (see Approved Approach).

## Out of Scope

- `test_linux.rs`'s `Compiler` trait, `test_in_vm.rs`, `test_image.rs` — untouched.
- `xtask/src/main.rs`'s top-level `fn main() -> Result<()>` — stays `anyhow::Result`,
  not changed to `miette::Result` (that would cascade into every xtask call site;
  explicitly rejected during brainstorm as too large/risky for a quick win during
  the stabilization freeze).
- The other two quick wins from this session's ideation (regression test for
  `handle_update` restart gap #178; xtask path-resolution consolidation) — separate,
  not part of this design.
- `update.rs:2` (#178) and `utils.rs:3` TODO comments — left untouched, already accurate.
- Fallback musl-gcc cross-linking logic (present in `test_linux::ZigbuildCompiler` for
  CI-environment robustness) — not needed here per Data Flow above.

## Risk

- [ ] Breaking API changes: no
- [x] New external dependency: yes — `miette`, `thiserror` added to `xtask/Cargo.toml`
      (workspace-pinned versions, scoped carve-out from documented miette rollout plan)
- [ ] Feature flag required: no
