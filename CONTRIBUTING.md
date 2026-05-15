# Contributing to Minibox

## Feature Freeze

**A net-new surface freeze is currently in effect.**

No new public API surface, features, or adapters may be merged until all mandatory stability
gates pass on CI. This freeze is formal and operational — not advisory.

### What "passing" means

All 7 mandatory [GATE] items in [`docs/STABILITY_CHECKLIST.mbx.md`](docs/STABILITY_CHECKLIST.mbx.md)
must be green on the `develop` branch in CI simultaneously. The current blocking gate is:

- Gate 2: Handler coverage >= 80% (currently ~67.5%)

### Unlock condition

All 7 mandatory gates green in CI on `develop`. A maintainer will tag the freeze as lifted
in the tracking issue and update this file.

### What IS allowed during the freeze

- Bug fixes to existing behaviour
- Tests that increase coverage toward Gate 2
- Documentation updates
- Refactors that do not change public API surface
- Dependency updates (security patches, version bumps)
- CI and tooling improvements

### What is NOT allowed during the freeze

- New public API variants (new `DaemonRequest`/`DaemonResponse` variants, new domain traits)
- New features visible to users via the CLI or protocol
- New adapter implementations
- New crates added to the workspace
- Any change that widens the public surface of `minibox-core`

### Chain I issues are explicitly gated

Issues in the Chain I stabilization track (#94, #20, #83, and related) are blocked by this
freeze. They will not be merged until the unlock condition is satisfied, regardless of
implementation readiness. Do not open PRs for Chain I work during the freeze period.

---

## Development Workflow

See [`DEVELOPMENT.md`](DEVELOPMENT.md) for the canonical developer workflow, command
reference, and CI gate descriptions.

### Quick gates

```bash
cargo xtask pre-commit     # fmt + clippy + release build (macOS-safe)
cargo xtask test-unit      # cross-platform unit and conformance subset
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
- [ ] `cargo xtask test-unit` passes locally
- [ ] No new `.unwrap()` in production paths
- [ ] No new public API surface (during freeze)
- [ ] PR description references the issue being addressed
- [ ] Any unmet [ADVISORY] items are acknowledged with rationale in the PR description
