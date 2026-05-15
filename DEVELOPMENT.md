# Development Guide

Canonical developer workflow for minibox. See `CLAUDE.md` for architecture
details and `TESTING.md` for the full test strategy.

## Prerequisites

- Rust stable (edition 2024)
- [cargo-nextest](https://nexte.st/) for test filtering
- [just](https://github.com/casey/just) for convenience recipes
- [uv](https://docs.astral.sh/uv/) for Python script dependencies
- Linux + root for integration/e2e tests (unit tests run on macOS)

## Good to have

- [Nushell](https://www.nushell.sh/) is my default terminal shell and most scripts are written in `nu` then translated to `bash` shellscripts

## Runner Hierarchy

Minibox has two task runners. They are complementary, not competing:

| Runner  | Role                              | When to use                  |
| ------- | --------------------------------- | ---------------------------- |
| `xtask` | CI gates, canonical test suites   | Always for CI-critical paths |
| `just`  | Wraps xtask + convenience recipes | Day-to-day development       |

**Rule of thumb:** if a GitHub Actions workflow calls it, the source of truth
is `cargo xtask <command>`. `just` recipes delegate to xtask where possible.
`scripts/` contains AI agent tooling and one-off helpers -- not part of the
core build/test pipeline.

## Quick Start

```bash
# Install git hooks (pre-commit, pre-push, commit-msg) — run once after cloning
just install-hooks

# Build everything (release)
cargo build --release

# Format
cargo fmt --all

# Lint (all workspace crates, deny warnings)
cargo clippy --workspace -- -D warnings
# Read-only local verification gate
cargo xtask verify

# Borrow-reasoning fixtures only
cargo xtask borrow-fixtures

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
sudo ./target/release/mbx stop <container_id>
sudo ./target/release/mbx rm <container_id>
```

Set `RUST_LOG=debug` for verbose tracing output.

## Building

```bash
cargo build --release                # all crates
just build-release                   # optimised (macOS-safe)
just build-linux                     # static musl binary (auto-detects arch)
```

## Testing

### Unit tests (any platform)

```bash
cargo xtask test-unit        # canonical
just test-unit               # equivalent shorthand
```

### Borrow-reasoning fixtures (any platform)

```bash
cargo xtask borrow-fixtures  # standalone must-pass/must-fail Rust borrow examples
cargo xtask borrow fixtures  # equivalent alias
```

The fixture suite checks standalone Rust examples directly with `rustc`:
fixtures under `xtask/fixtures/borrow/pass` are `must-pass`, while fixtures under
`xtask/fixtures/borrow/fail` are `must-fail` with their declared `// expect: ...`
diagnostic snippets. The examples cover moves, shared and unique borrows,
disjoint field borrows, reborrowing, NLL last-use behavior, and conservative
branch joins.

### Integration tests (Linux + root)

```bash
just test-integration        # cgroup tests + native adapter isolation
```

### End-to-end tests (Linux + root)

```bash
just test-e2e                # single lifecycle test
just test-e2e-suite          # full daemon+CLI e2e suite
just test-e2e-vps            # run e2e suite on VPS via SSH
```

### Property-based tests (any platform)

```bash
cargo xtask test-property
```

### Adapter and CLI tests (any platform)

```bash
just test-adapters           # Colima + handler adapter swap tests
just test-cli-subprocess     # CLI subprocess integration tests
```

### VM tests

```bash
just test-linux              # dogfood: build image + run tests in container
```

### Coverage

```bash
just coverage                # HTML report at target/llvm-cov/html/
```

### Preflight / Doctor (canonical entry points)

```bash
cargo xtask doctor           # CANONICAL: tool checks + env + Linux system caps
mbx doctor                   # adapter diagnostics + delegates to cargo xtask doctor
```

`cargo xtask doctor` is the authoritative preflight command. It checks:
- Required tools on PATH: `cargo`, `just`, `rustup`, `cargo-nextest`
- Advisory tools: `gh`, `op` (warn, not fail)
- `CARGO_TARGET_DIR` env var (advisory)
- Linux-only: cgroups v2 unified hierarchy, overlay FS, kernel >= 5.0

`mbx doctor` runs `cargo xtask doctor` first, then shows adapter suite diagnostics
(which adapter is compiled in and which would be selected by the current environment).

`scripts/preflight.nu` is a lightweight SessionStart hook — it runs at shell startup
to surface obvious missing deps. It is not a substitute for `cargo xtask doctor`.

### Full pipeline

```bash
just test-all                # nuke state -> doctor -> unit + integration + e2e -> nuke
```

## Benchmarks

```bash
cargo xtask bench            # run locally, save to bench/results/
just bench-sync              # sync VPS results to local jsonl
just flamegraph [suite]      # profile with samply/flamegraph
just bench-agent report      # AI bench analysis
```

## CI Gates

Local validation should match CI. The commands that matter:

1. **Read-only local gate:** `cargo xtask verify`
2. **Before every commit:** `cargo xtask pre-commit`
3. **Before every push:** `cargo xtask prepush`

GitHub Actions (`pr.yml` + `merge.yml`) runs the same xtask commands plus
`cargo deny`, `cargo audit`, and `cargo machete` on the `next` and `stable` branches.

## Environment Variables

| Variable               | Purpose                                          | Default                             |
| ---------------------- | ------------------------------------------------ | ----------------------------------- |
| `MINIBOX_ADAPTER`      | Adapter suite: native, gke, colima, smolvm, krun | `smolvm` (macOS) / `native` (Linux) |
| `MINIBOX_DATA_DIR`     | Image/container storage                          | `/var/lib/minibox` (root)           |
| `MINIBOX_RUN_DIR`      | Socket/runtime directory                         | `/run/minibox`                      |
| `MINIBOX_SOCKET_PATH`  | Unix socket path                                 | `$MINIBOX_RUN_DIR/miniboxd.sock`    |
| `MINIBOX_CGROUP_ROOT`  | Cgroup root for containers                       | systemd slice path                  |
| `MINIBOX_NETWORK_MODE` | Network mode: none, bridge                       | `none`                              |
| `RUST_LOG`             | Tracing verbosity (debug, info, warn, etc)       | unset                               |

## Cleanup

```bash
just clean-artifacts         # remove non-critical build outputs
just clean-test              # remove test binaries
just clean-stale [days]      # remove files older than N days (default: 7)
cargo xtask nuke-test-state  # kill orphans, unmount overlays, clean cgroups
```

## scripts/ Directory

The `scripts/` directory contains AI agent tooling (council analysis,
AI-assisted code review, test generation) and operational helpers (VM setup,
daemon start, cgroup test harness). These are **not** part of the core
build/test pipeline. All Python scripts use `uv run` with PEP 723 inline
deps. See `just --list` for recipes that wrap them.
