# Stability Checklist

Before adding a new crate, wiring a new adapter suite, or shipping a major feature, confirm
that each item below is true. The intent is to keep the core runtime trustworthy as the agent
and platform surface grows.

---

## Runtime Integrity

- [x] **Protocol types have a single source of truth.** `DaemonRequest`/`DaemonResponse` are
  defined only in `minibox-core/src/protocol.rs`. `mbx` re-exports via
  `pub use minibox_core::protocol`. Wire format snapshot tests in `minibox-core` pin
  serialization. Add new variants only to `minibox-core/src/protocol.rs`.

- [ ] **Handler coverage >= 80% (function).** Current baseline: ~67.5% function / ~55% line
  in `daemonbox/src/handler.rs`. Run `cargo xtask prepush` for the llvm-cov report. Error
  paths (pull failure, empty image, registry unreachable) have the highest ROI.

- [ ] **All wired adapters have at least one integration test.** Unit tests with mocks are
  necessary but not sufficient — each `MINIBOX_ADAPTER` value accepted by the daemon must
  have at least one test that exercises the real adapter path.

- [ ] **`cargo xtask pre-commit` passes on macOS.** This gate runs `cargo fmt --check`,
  clippy (all crates), and `cargo build --release`. No warnings allowed.

- [ ] **`cargo xtask test-unit` passes.** ~300+ unit + conformance tests via nextest.

## Security Gate

- [ ] **Path validation is in place for all external inputs.** Any new code that touches the
  filesystem using paths from user input, tar entries, or registry data must go through
  `validate_layer_path()` or equivalent canonicalize + prefix-check.

- [ ] **No `.unwrap()` in production code paths.** Use `.context("description")?` instead.
  Test code may use `.expect("reason")`.

- [ ] **`SO_PEERCRED` auth is unmodified.** The UID == 0 check in `daemonbox/server.rs` must
  run before any request processing. Do not weaken or gate it behind a feature flag.

- [ ] **`unsafe` blocks have documented SAFETY comments.** Every `unsafe {}` must explain
  the invariant the caller upholds.

- [ ] **`cargo deny check` passes.** License and advisory audit must be clean.

## Documentation Matches Reality

- [ ] **CLAUDE.md "Current Limitations" section is accurate.** Update it whenever a listed
  limitation is removed or a new one is introduced.

- [ ] **README.md feature list matches `docs/FEATURE_MATRIX.md`.** Do not add a feature to
  the README "Features" section until it appears as `✓` or `~` in the matrix.

- [ ] **`docs/FEATURE_MATRIX.md` is updated.** When a feature moves from stub to
  experimental, or from experimental to shipped, update the matrix before merging.

## Agent Feature Gating

Agent-facing features (MCP control surface, sandboxed execution, CI dogfooding) must wait
until **all** of the following are true:

- [x] Protocol types consolidated — single `DaemonRequest` source (minibox-core, #122/#128).
- [ ] Handler coverage >= 80% (function coverage, measured by llvm-cov).
- [ ] Auth policy gate implemented for privileged operations (bind mounts, privileged mode).
- [ ] Conformance suite passes for the targeted adapter suite (`cargo xtask test-conformance`).
- [ ] Feature matrix entry exists and is marked at least `~` (experimental).

---

## How to Use This Checklist

Run through this list before opening a PR that:

- Adds a new crate to the workspace
- Wires a new `MINIBOX_ADAPTER` value
- Adds a new `DaemonRequest` variant
- Exposes a new CLI command
- Introduces a new `unsafe` block

Not every item applies to every change. Use judgment — the goal is that the checklist
prevents known categories of regression, not that it blocks small fixes.
