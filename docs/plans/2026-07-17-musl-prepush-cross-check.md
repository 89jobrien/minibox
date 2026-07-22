# Plan: musl release cross-check in prepush gate

## Goal

Add a `cargo zigbuild` release cross-build of `miniboxd`+`mbx` for
`x86_64-unknown-linux-musl` to the `prepush` xtask gate, so musl-specific
release-build breakage is caught before push instead of in CI.

Design doc: `docs/designs/2026-07-17-musl-prepush-cross-check-design.md`

## Architecture

- Crates affected: `xtask` only (leaf binary crate, no downstream consumers)
- New types: `MuslCrossCheckError` (private, `xtask/src/gates.rs`)
- New functions: `cross_check_musl_release` (private, `xtask/src/gates.rs`)
- Data flow: `prepush()` gate invocation -> `cross_check_musl_release(&sh)` shells
  out to `cargo zigbuild --release -p miniboxd -p mbx --target x86_64-unknown-linux-musl`
  -> on success, falls through to the existing native release build/nextest/conformance
  steps unchanged -> on failure, renders `MuslCrossCheckError` via miette's fancy
  handler then converts to `anyhow::Error` to satisfy `prepush()`'s existing
  `Result<()>` chain

## Tech Stack

- Rust edition 2024, `xtask` binary crate
- New dependencies: `miette = { workspace = true }` (already pinned workspace-wide
  at `miette = { version = "7", features = ["fancy"] }`), `thiserror = { workspace = true }`
  (already pinned at `thiserror = "2"`) — both added to `xtask/Cargo.toml` only.
  This is a deliberate, scoped exception: `xtask` currently depends solely on
  `anyhow` for errors, and the project's documented miette rollout plan
  (`docs/plans/2026-07-07-structured-errors-miette.md`) explicitly excludes
  `xtask` from scope. Approved as a carve-out for this one error path only.
- `cargo-zigbuild` must be installed on the dev machine (confirmed present at
  `/Users/joe/.cargo/bin/cargo-zigbuild`) and the `x86_64-unknown-linux-musl`
  rustup target must be installed (confirmed **not yet installed** — added as a
  prerequisite step in Task 3).

## Tasks

### Task 1: Add miette + thiserror to xtask/Cargo.toml

**Crate**: `xtask`
**File(s)**: `xtask/Cargo.toml`

1. Open `xtask/Cargo.toml`, locate the `[dependencies]` section (currently ends
   with `xshell = "0.2"`).

2. Add two lines directly below `xshell = "0.2"`:

   ```toml
   miette = { workspace = true }
   thiserror = { workspace = true }
   ```

3. Verify:

   ```
   cargo check -p xtask
   ```

   Expected: compiles clean (no new code uses these deps yet, so this just
   confirms the workspace-pinned versions resolve without conflict).

4. Run: `git branch --show-current`
   Verify output is not `main`. Stop immediately if it is.
   Commit: `git commit -m "chore(xtask): add miette+thiserror deps for musl cross-check diagnostic"`

### Task 2: Add MuslCrossCheckError with a diagnostic-shape test

**Crate**: `xtask`
**File(s)**: `xtask/src/gates.rs`
**Run**: `cargo nextest run -p xtask -- musl_cross_check_error_has_diagnostic_shape`

1. Write failing test. In `xtask/src/gates.rs`, inside the existing `mod tests`
   block (starts at line 791, after the `use super::parse_handler_fn_coverage;`
   line), add:

   ```rust
   #[test]
   fn musl_cross_check_error_has_diagnostic_shape() {
       use miette::Diagnostic;
       let err = super::MuslCrossCheckError;
       assert_eq!(
           err.code().map(|c| c.to_string()),
           Some("xtask::musl_cross_check".to_string())
       );
       assert!(err.help().is_some(), "expected a help string on MuslCrossCheckError");
       assert_eq!(
           err.to_string(),
           "x86_64-unknown-linux-musl release cross-build failed"
       );
   }
   ```

   Run: `cargo nextest run -p xtask -- musl_cross_check_error_has_diagnostic_shape`
   Expected: FAIL (compile error — `MuslCrossCheckError` does not exist yet)

2. Implement. Above `pub fn prepush(sh: &Shell) -> Result<()> {` (currently line 179),
   add:

   ```rust
   #[derive(Debug, thiserror::Error, miette::Diagnostic)]
   #[error("x86_64-unknown-linux-musl release cross-build failed")]
   #[diagnostic(
       code(xtask::musl_cross_check),
       help("run `rustup target add x86_64-unknown-linux-musl`, or install cargo-zigbuild if missing")
   )]
   struct MuslCrossCheckError;
   ```

3. Verify:

   ```
   cargo nextest run -p xtask -- musl_cross_check_error_has_diagnostic_shape   → PASS
   cargo clippy -p xtask -- -D warnings                                        → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is not `main`. Stop immediately if it is.
   Commit: `git commit -m "feat(xtask): add MuslCrossCheckError diagnostic type"`

### Task 3: Implement cross_check_musl_release and wire into prepush

**Crate**: `xtask`
**File(s)**: `xtask/src/gates.rs`

1. Prerequisite (one-time, not part of the commit): install the missing rustup
   target so the success path can be verified locally:

   ```
   rustup target add x86_64-unknown-linux-musl
   ```

2. Implement. Replace the stale TODO block currently at lines 187-189
   (`// TODO: add \`cargo check --target x86_64-unknown-linux-musl\` here to catch ...`
   through `// the 2026-05-22 cluster of 5 iterative CI-fix-push cycles. See mistakes.md.`)
   with a call to a new helper, and add that helper directly above `pub fn prepush`
   (below the `MuslCrossCheckError` struct added in Task 2):

   ```rust
   /// Cross-compile `miniboxd` + `mbx` in release mode for the VPS deploy
   /// target (`x86_64-unknown-linux-musl`) via `cargo zigbuild`. Hard-fails
   /// with a rendered miette diagnostic on any failure (missing target,
   /// missing zigbuild, or actual compile/link error).
   fn cross_check_musl_release(sh: &Shell) -> Result<()> {
       cmd!(
           sh,
           "cargo zigbuild --release -p miniboxd -p mbx --target x86_64-unknown-linux-musl"
       )
       .run()
       .inspect_err(|_| {
           eprintln!("{:?}", miette::Report::new(MuslCrossCheckError));
       })
       .context("musl release cross-check failed")?;
       Ok(())
   }
   ```

   Then update the `gated(GateId::Prepush, &root, move || { ... })` closure body
   in `prepush()` so `cross_check_musl_release(&sh)?;` is the first line, directly
   before the existing:

   ```rust
   cmd!(
       sh,
       "cargo build --release -p minibox -p minibox-macros -p mbx -p minibox-core -p miniboxd"
   )
   .run()
   .context("release build failed")?;
   ```

3. Verify:

   ```
   cargo check -p xtask                     → compiles clean
   cargo clippy -p xtask -- -D warnings     → zero warnings
   cargo xtask prepush                      → runs cross_check_musl_release first;
                                               confirm it prints
                                               "compiling  miniboxd → x86_64-unknown-linux-musl"
                                               and "compiling  mbx → x86_64-unknown-linux-musl"
                                               (or equivalent zigbuild output) before
                                               proceeding to the native release build
   ```

   To verify the failure path renders the diagnostic correctly, temporarily rename
   the target string to an invalid triple (e.g. `x86_64-unknown-linux-muslx`) in
   `cross_check_musl_release`, re-run `cargo xtask prepush`, and confirm the
   fancy-rendered `xtask::musl_cross_check` diagnostic with its help text prints to
   stderr before the command exits non-zero. **Revert the typo before committing.**

4. Run: `git branch --show-current`
   Verify output is not `main`. Stop immediately if it is.
   Commit: `git commit -m "feat(xtask): add musl release cross-check to prepush gate"`

## Out of Scope

(carried over from the design doc)

- `test_linux.rs`'s `Compiler` trait, `test_in_vm.rs`, `test_image.rs` — untouched.
- `xtask/src/main.rs`'s `fn main() -> Result<()>` — stays `anyhow::Result`, not
  changed to `miette::Result`.
- The other two quick wins from this session's ideation (regression test for
  `handle_update` restart gap #178; xtask path-resolution consolidation) — separate
  plans, not part of this one.
- `update.rs:2` (#178) and `utils.rs:3` TODO comments — left untouched.
- Fallback musl-gcc cross-linking logic (present in `test_linux::ZigbuildCompiler`
  for CI-environment robustness) — not needed for this local dev-machine gate.
