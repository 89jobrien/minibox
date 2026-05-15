---
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
cargo fmt --all --check
cargo clippy -p mbx -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -- -D warnings
cargo xtask test-unit
```

### 2. Linux Quality Gates

```bash
cargo xtask test-unit
cargo xtask test-property
just test-integration    # cgroup tests, requires root
just test-e2e            # daemon+CLI tests, requires root
just doctor              # preflight capability check
```

### 3. Security Checks

```bash
cargo audit          # known vulnerability scan
cargo deny check     # license + ban check; Gitea CI also runs this
```

### 4. Benchmark Baseline

```bash
cargo xtask bench

# Verify results saved
cat bench/results/latest.json | head -20

# Check for regressions vs previous
git diff bench/results/bench.jsonl | tail -30
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

Edit `Cargo.toml` at the workspace root:

```toml
[workspace.package]
version = "0.5.0"
```

Add a section to `CHANGELOG.md`:

```markdown
## [0.5.0] - 2026-03-21

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

./target/release/miniboxd --version
./target/release/minibox --version

# Re-run quality gates after the bump
cargo fmt --all --check
cargo clippy -p mbx -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -- -D warnings
cargo xtask test-unit
```

### Step 4: Commit

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md

git commit -m "chore(release): bump version to v0.5.0

- Updated workspace version in Cargo.toml
- Updated CHANGELOG.md with release notes
- All quality gates pass
- Benchmarks stable

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

### Step 5: Create Annotated Tag

```bash
git tag -a v0.5.0 -m "Release v0.5.0

Added:
- exec command for running commands in existing containers
- winbox adapter suite for Windows HCS

Fixed:
- Absolute symlink rewrite in layer.rs
- cgroup.procs PID 0 validation

Security:
- Stricter path validation in overlay setup"
```

### Step 6: Push to Both Remotes

```bash
# Gitea — self-hosted, primary CI
git push gitea main
git push gitea v0.5.0

# GitHub — macOS Actions CI
git push github main
git push github v0.5.0
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

Expected jobs: `cargo fmt --all --check`, clippy on all crates, `cargo xtask test-unit`.

## Rollback

### Option 1: Patch Release

Preferred for bugs found after tagging.

```bash
git checkout -b hotfix/v0.5.1
# apply fix
cargo xtask test-unit
just test-integration
git commit -m "fix: ..."
# then follow release steps above for v0.5.1
```

### Option 2: Revert Tag

Last resort.

```bash
git tag -d v0.5.0
git push gitea :refs/tags/v0.5.0
git push github :refs/tags/v0.5.0

git revert HEAD
git push gitea main
git push github main
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
cargo clippy -p mbx -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -- -D warnings
```

Fix all warnings, then re-tag.

### Version mismatch in Cargo.lock

```bash
cargo update --workspace
cargo build --release
./target/release/minibox --version
```

### Benchmark results missing

```bash
cargo xtask bench
ls -la bench/results/
```

Both `bench.jsonl` and `latest.json` must exist and be in sync before releasing.

## Security Pre-Release Checklist

- [ ] No secrets committed
- [ ] `cargo audit` clean
- [ ] `cargo deny check` clean
- [ ] Path validation present on all user-input handling
- [ ] `SO_PEERCRED` check not weakened in server.rs
- [ ] Tar extraction security checks intact in layer.rs — `..` components, device nodes, setuid bits
- [ ] Resource limits enforced — max manifest 10 MB, max layer 1 GB

## Release Cadence

- **PATCH**: As needed for security fixes; same-day turnaround for critical CVEs
- **MINOR**: When a new adapter suite or container capability is complete
- **MAJOR**: Protocol-breaking changes only
