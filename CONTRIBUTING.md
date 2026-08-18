# Contributing to Minibox

## Feature Freeze (lifted 2026-08-18)

The net-new surface freeze declared here on 2026-05-14 (issue #127) was **lifted on
2026-08-18**: all six mandatory [GATE] items in
[`docs/core/STABILITY_CHECKLIST.mbx.md`](docs/core/STABILITY_CHECKLIST.mbx.md) were
verified green simultaneously on the promotion path, including Linux
integration/e2e evidence. The lift record and evidence live in that document's
"Freeze Status" section; issue #127 records the decision.

What replaces the freeze:

- New crates and new public surface follow the standing **Stabilization Policy**
  in [`docs/core/CRATE_TIERS.mbx.md`](docs/core/CRATE_TIERS.mbx.md) — the gate
  criteria did not go away, they are now the promotion bar rather than a blanket
  block.
- Chain I stabilization-track issues (#94, #20, #83, and related) are unblocked.
- The mandatory gates remain enforced in CI (`stability-gates.yml`,
  `protocol-drift.yml`, `pr.yml`/`merge.yml`) — a regression in any gate is a
  merge blocker exactly as during the freeze.

---

## Development Workflow

See [`DEVELOPMENT.md`](DEVELOPMENT.md) for the canonical developer workflow, command
reference, and CI gate descriptions.

### Quick gates

```bash
cargo xtask pre-commit     # fmt + clippy + release build (macOS-safe)
cargo xtask test unit      # cross-platform unit and conformance subset
cargo deny check           # license + advisory audit
```

### Commit style

```
type(scope): short imperative description (#issue)
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `ci`.

Scope is the crate or area: `minibox-core`, `miniboxd`, `minibox-cli`, `xtask`, `freeze`, etc.

---

## Code Standards

See [`.claude/rules/rust-patterns.md`](.claude/rules/rust-patterns.md) for the full set of
non-negotiable Rust patterns enforced in this repository. Key rules:

- No `.unwrap()` in production paths — use `.context("description")?`
- All user-supplied paths go through `validate_layer_path()` before filesystem access
- `fork()`/`clone()`/`exec` operations must run in `tokio::task::spawn_blocking`
- Every `unsafe` block requires a `// SAFETY:` comment explaining the invariant

---

## Pull Request Checklist

Before opening a PR, confirm:

- [ ] `cargo xtask pre-commit` passes locally
- [ ] `cargo xtask test unit` passes locally
- [ ] No new `.unwrap()` in production paths
- [ ] New public API surface follows the Stabilization Policy in `docs/core/CRATE_TIERS.mbx.md`
- [ ] PR description references the issue being addressed
- [ ] Any unmet [ADVISORY] items are acknowledged with rationale in the PR description
