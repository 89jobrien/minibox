# Development Guide

Canonical developer workflow for minibox. See `CLAUDE.md` for architecture
details and `TESTING.md` for the full test strategy.

## Prerequisites

- Rust stable (edition 2024)
- [cargo-nextest](https://nexte.st/) for test filtering
- [just](https://github.com/casey/just) for convenience recipes
- Linux + root for integration/e2e tests (unit tests run on macOS)

## Runner Hierarchy

Minibox has three task runners. They are complementary, not competing:

| Runner   | Role                              | When to use                    |
| -------- | --------------------------------- | ------------------------------ |
| `xtask`  | CI gates, canonical test suites   | Always for CI-critical paths   |
| `just`   | Wraps xtask + convenience recipes | Day-to-day development         |
| `mise`   | Interactive/ops tasks             | Demos, VPS ops, git helpers    |

**Rule of thumb:** if a GitHub Actions workflow calls it, the source of truth
is `cargo xtask <command>`. `just` recipes delegate to xtask where possible.
`scripts/` contains AI agent tooling and one-off helpers -- not part of the
core build/test pipeline.

## Quick Start

```bash
# Build everything (release)
cargo build --release

# Format
cargo fmt --all

# Lint (all workspace crates, deny warnings)
cargo clippy --workspace -- -D warnings

# Unit tests (any platform)
cargo xtask test-unit

# Pre-commit gate (format check + clippy + release build)
cargo xtask pre-commit

# Pre-push gate (nextest + llvm-cov coverage)
cargo xtask prepush
```

## Running the Daemon

```bash
# Start daemon (Linux, requires root)
sudo ./target/release/miniboxd

# CLI commands (daemon must be running)
sudo ./target/release/mbx pull alpine
sudo ./target/release/mbx run alpine -- /bin/echo "Hello"
sudo ./target/release/mbx ps
```

Set `RUST_LOG=debug` for verbose tracing output.

## Testing

### Unit tests (any platform)

```bash
cargo xtask test-unit        # canonical
just test-unit               # equivalent shorthand
```

### Integration tests (Linux + root)

```bash
just test-integration        # cgroup tests + native adapter isolation
```

### End-to-end tests (Linux + root)

```bash
just test-e2e                # single lifecycle test
just test-e2e-suite          # full daemon+CLI e2e suite
```

### Property-based tests (any platform)

```bash
cargo xtask test-property
```

### Coverage

```bash
just coverage                # HTML report at target/llvm-cov/html/
```

### Host capability check

```bash
just doctor                  # reports kernel features, cgroups, overlay support
```

## CI Gates

Local validation should match CI. The two commands that matter:

1. **Before every commit:** `cargo xtask pre-commit`
2. **Before every push:** `cargo xtask prepush`

GitHub Actions (`.github/workflows/ci.yml`) runs the same xtask commands plus
`cargo deny` and `cargo audit` on the `next` and `stable` branches.

## Environment Variables

| Variable              | Purpose                                    | Default                          |
| --------------------- | ------------------------------------------ | -------------------------------- |
| `MINIBOX_ADAPTER`     | Adapter suite: native, gke, colima, vz     | `native`                         |
| `MINIBOX_DATA_DIR`    | Image/container storage                    | `/var/lib/minibox` (root)        |
| `MINIBOX_RUN_DIR`     | Socket/runtime directory                   | `/run/minibox`                   |
| `MINIBOX_SOCKET_PATH` | Unix socket path                           | `$MINIBOX_RUN_DIR/miniboxd.sock` |
| `MINIBOX_CGROUP_ROOT` | Cgroup root for containers                 | systemd slice path               |
| `RUST_LOG`            | Tracing verbosity (debug, info, warn, etc) | unset                            |

## scripts/ Directory

The `scripts/` directory contains AI agent tooling (council analysis,
AI-assisted code review, test generation) and operational helpers (VM setup,
daemon start, cgroup test harness). These are **not** part of the core
build/test pipeline. See `just --list` for recipes that wrap them.
