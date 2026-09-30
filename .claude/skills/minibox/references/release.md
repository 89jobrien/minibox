---
name: release
description: Release workflow for minibox — quality gates, version bump, changelog, git tag, push to Gitea + GitHub
---

# Release

Systematic release workflow for minibox: pre-release quality gates, version bump across workspace, changelog update, git tag, and push to trigger CI.

## When to Use

- When ready to cut a new version
- After a feature or fix is complete and all tests pass
- To automate the release checklist before tagging

## Pre-Release Checklist

### 1. macOS Quality Gates

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

### 2. Linux Quality Gates

```bash
cargo xtask test unit
cargo xtask test property
just test-integration    # cgroup tests, requires root
just test-e2e            # protocol e2e tests, no root required
just doctor              # preflight capability check
```

### 3. Security Checks

```bash
cargo audit          # known vulnerability scan
cargo deny check     # license + ban check; Gitea CI also runs this
```

### 4. Benchmark Baseline

```bash
just bench
just bench-check
```

### 5. Clean Working Tree

```bash
git status  # should show nothing to commit
```

## Release Steps

### Step 1: Determine Version Bump

Semantic versioning — MAJOR.MINOR.PATCH:

- **MAJOR**: Breaking protocol changes, removed commands, incompatible CLI flags
- **MINOR**: New features — new adapter suite, new CLI command, new container capability
- **PATCH**: Bug fixes, security patches, performance improvements

Examples:

- New `exec` command → MINOR bump, v0.4.0 → v0.5.0
- Security fix in tar extraction → PATCH bump, v0.4.0 → v0.4.1
- Protocol-breaking change → MAJOR bump, v0.4.0 → v1.0.0

### Step 2: Update Version

Apply the selected bump and let xtask update the workspace version and changelog:

```bash
cargo xtask bump <patch|minor|major> --changelog
```

Read the resulting `[workspace.package].version` from `Cargo.toml`; use that exact
value as `<version>` in the remaining commands. Verify the generated section in
`CHANGELOG.md`:

Update the `workspace_version` fact markers in `docs/core/ARCHITECTURE.mbx.md`
and `docs/core/CRATE_INVENTORY.mbx.md` to the same version before running gates.

```markdown
## [<version>] - <release-date>

### Added

- `exec` command: run commands in existing containers
- `winbox` adapter suite for Windows HCS

### Fixed

- Absolute symlink rewrite in layer.rs for busybox applet links
- cgroup.procs PID 0 validation

### Security

- Stricter path validation in overlay filesystem setup
- SO_PEERCRED check rejects non-root UIDs before any deserialization

### Changed

- Benchmark results saved to `bench/results/bench.jsonl` as append-only history
```

### Step 3: Build and Verify

```bash
cargo build --release

./target/release/mbx --version

# Re-run quality gates after the bump
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

### Step 4: Commit

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md docs/core/ARCHITECTURE.mbx.md docs/core/CRATE_INVENTORY.mbx.md

git commit -m "chore(release): bump version to v<version>

- Updated workspace version in Cargo.toml
- Updated CHANGELOG.md with release notes
- All quality gates pass
- Benchmarks stable

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

### Step 5: Create Annotated Tag

```bash
git tag -a v<version> -m "Release v<version>

Added:
- exec command for running commands in existing containers
- winbox adapter suite for Windows HCS

Fixed:
- Absolute symlink rewrite in layer.rs
- cgroup.procs PID 0 validation

Security:
- Stricter path validation in overlay setup"
```

### Step 6: Promote and Push the Tag

```bash
# Promote through the protected branch pipeline; do not push main directly.
cargo xtask promote --from release --to main

# Publish the annotated tag to both mirrors after main contains the release commit.
git push gitea v<version>
git push origin v<version>
```

## CI Verification

### Gitea CI

```bash
mise run ci
```

Gitea runs `cargo deny check` and `cargo audit` only — no compilation on the VPS.

### GitHub Actions CI

```bash
gh run list --limit 3
gh run watch
```

Expected jobs include formatting, workspace clippy, nextest, stability, and actionlint gates.

## Rollback

### Option 1: Patch Release

Preferred for bugs found after tagging.

```bash
git checkout -b hotfix/v<patch-version>
# apply fix
cargo xtask test unit
just test-integration
git commit -m "fix: ..."
# then follow the release steps above with the resulting patch version
```

### Option 2: Revert Tag

Last resort.

```bash
git tag -d v<version>
git push gitea :refs/tags/v<version>
git push origin :refs/tags/v<version>

git revert HEAD
# Commit the revert on a hotfix/release branch, then use the normal promotion pipeline.
cargo xtask promote --from release --to main
```

## Common Issues

### Gitea CI fails on deny/audit

```bash
cargo deny check licenses 2>&1 | grep ERROR
# Workspace crates need `license = "MIT"` in their Cargo.toml or deny.toml
# rejects them as unlicensed

cargo audit
# cargo update <crate> to pick up a patched version
```

### Clippy fails on GitHub Actions

Run the exact command locally to reproduce:

```bash
cargo clippy --workspace -- -D warnings
```

Fix all warnings, then re-tag.

### Version mismatch in Cargo.lock

```bash
cargo update --workspace
cargo build --release
./target/release/mbx --version
```

### Benchmark results missing

```bash
cargo xtask bench
ls -la bench/results/
```

The generated result snapshot and selected environment baseline must be available before release.

## Security Pre-Release Checklist

- [ ] No secrets committed
- [ ] `cargo audit` clean
- [ ] `cargo deny check` clean
- [ ] Path validation present on all user-input handling
- [ ] `SO_PEERCRED` check not weakened in server.rs
- [ ] Tar extraction security checks intact in layer.rs — `..` components, device nodes, setuid bits
- [ ] Resource limits enforced — max manifest 10 MiB, max layer 10 GiB, total image 50 GiB

## Release Cadence

- **PATCH**: As needed for security fixes; same-day turnaround for critical CVEs
- **MINOR**: When a new adapter suite or container capability is complete
- **MAJOR**: Protocol-breaking changes only
