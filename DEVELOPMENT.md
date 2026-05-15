# Development Guide

Canonical developer workflow for minibox. See `CLAUDE.md` for architecture
details, `TESTING.md` for the full test strategy, and `docs/SUPPORT_TIERS.mbx.md` for crate and
adapter support-tier definitions.

## Quick Start for New Contributors

Three commands cover 95% of daily development:

```bash
cargo xtask pre-commit   # before every commit: fmt-check + clippy + release build
cargo xtask test-unit    # run all unit + conformance tests (any platform)
cargo xtask prepush      # before every push: nextest suite + coverage
```

Install git hooks once after cloning:

```bash
just install-hooks
```

See the [Command Reference](#command-reference) table below for the full list.

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

### Adapter Selection

Adapter selection is handled entirely inside `miniboxd` — no wrapper script or
external env setup is required. The daemon reads `MINIBOX_ADAPTER` at startup and
applies its own fallback logic (see `crates/miniboxd/src/adapter_registry.rs`):

- **Unset** (default): tries `smolvm`; falls back to `krun` if the `smolvm`
  binary is not on `PATH`.
- **Explicit** (`MINIBOX_ADAPTER=<name>`): uses the named adapter as-is, no
  fallback.

```bash
# Auto-select (smolvm → krun fallback)
sudo ./target/release/miniboxd

# Pin to a specific adapter
sudo MINIBOX_ADAPTER=krun ./target/release/miniboxd
sudo MINIBOX_ADAPTER=native ./target/release/miniboxd   # Linux + root only

# Convenience scripts (pass --adapter flag or MINIBOX_ADAPTER)
nu scripts/start-daemon.nu --adapter krun
./scripts/start-daemon.sh --adapter native

# Inspect compiled adapters
sudo ./target/release/mbx doctor
```

Do **not** set `MINIBOX_ADAPTER` inside start scripts or systemd units to
hard-code an adapter — this bypasses the smolvm→krun fallback and will fail
if the named adapter binary is absent.

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
| `MINIBOX_ADAPTER`      | Adapter suite: native, gke, colima, smolvm, krun | auto: smolvm, fallback krun         |
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

## Command Reference

All commands listed here exist in either `cargo xtask` or `just`. Commands
marked _(Linux/root)_ require a Linux host with root privileges.

### Quality Gates

| Task                                  | Command                            | When to use                              |
| ------------------------------------- | ---------------------------------- | ---------------------------------------- |
| Before every commit                   | `cargo xtask pre-commit`           | fmt-check + clippy + release build       |
| Before every push                     | `cargo xtask prepush`              | nextest suite + coverage check           |
| Read-only local verification          | `cargo xtask verify`               | fmt + clippy + borrow fixtures + docs    |
| Auto-fix formatting/clippy            | `cargo xtask fix`                  | mutates files; review diff after         |
| Lint only (no build)                  | `cargo xtask lint`                 | fmt-check + clippy (CI lint gate)        |

### Testing

| Task                                  | Command                            | When to use                              |
| ------------------------------------- | ---------------------------------- | ---------------------------------------- |
| Unit + conformance tests              | `cargo xtask test-unit`            | Any platform; no root required           |
| Property-based tests                  | `cargo xtask test-property`        | Any platform; proptest suite             |
| Borrow-reasoning fixtures             | `cargo xtask borrow-fixtures`      | Standalone rustc must-pass/must-fail     |
| Protocol e2e tests                    | `cargo xtask test-e2e`             | Any platform; no root required           |
| Cgroup integration tests              | `just test-integration`            | Linux + root; cgroup v2                  |
| Full-stack system tests               | `cargo xtask test-system-suite`    | Linux + root; daemon + CLI               |
| Sandbox contract tests                | `cargo xtask test-sandbox`         | Linux + root; requires Docker Hub        |
| Adapter isolation tests               | `just test-adapters`               | Any platform                             |
| CLI subprocess tests                  | `just test-cli-subprocess`         | Any platform; builds mbx first           |
| Linux dogfood (build + run in VM)     | `cargo xtask test-linux`           | macOS + smolvm; runs suite in container  |
| Full pipeline                         | `just test-all`                    | Linux + root; nuke → all tests → nuke   |
| Remote VPS e2e                        | `just test-e2e-vps`                | Runs test-system-suite on VPS over SSH   |
| HTML coverage report                  | `just coverage`                    | Any platform; opens target/llvm-cov/     |

### Codebase Integrity Checks

| Task                                  | Command                                    | When to use                          |
| ------------------------------------- | ------------------------------------------ | ------------------------------------ |
| Detect stale crate/binary names       | `cargo xtask check-stale-names`            | After renames; CI gate               |
| Verify protocol contract hashes       | `cargo xtask check-protocol-drift`         | After protocol.rs changes            |
| Update protocol hash baseline         | `cargo xtask check-protocol-drift --update`| After intentional protocol changes   |
| Verify HandlerDependencies site count | `cargo xtask check-protocol-sites`         | After adding/removing handler sites  |
| Scan for `.unwrap()` in production    | `cargo xtask check-no-unwrap`              | Advisory; use `--strict` to fail     |
| Verify adapter test coverage          | `cargo xtask check-adapter-coverage`       | After adding a new adapter           |
| Check for tracked generated artifacts | `cargo xtask check-repo-clean`             | Before PRs                           |
| Lint docs frontmatter                 | `cargo xtask lint-docs`                    | After editing docs/superpowers/      |

### Build

| Task                                  | Command                            | When to use                              |
| ------------------------------------- | ---------------------------------- | ---------------------------------------- |
| Debug build (all crates)              | `cargo build`                      | Fast iteration                           |
| Release build (all crates)            | `cargo build --release`            | Pre-deployment check                     |
| Optimised macOS-safe build            | `just build-release`               | macOS dev                                |
| Static musl Linux binary              | `just build-linux`                 | Cross-compile for VPS deployment         |
| Build + load test image               | `cargo xtask build-test-image`     | Required before `test-linux`             |

### Cleanup

| Task                                  | Command                            | When to use                              |
| ------------------------------------- | ---------------------------------- | ---------------------------------------- |
| Remove non-critical build outputs     | `cargo xtask clean-artifacts`      | After a release build                    |
| Kill orphans, unmount overlays        | `cargo xtask nuke-test-state`      | After failed tests leave state behind    |
| Full cargo clean                      | `just clean`                       | Nuclear option                           |
| Remove stale build artifacts          | `just clean-stale [days]`          | Reclaim disk (default: 7 days)           |

### Repo Context & Orchestration

| Task                                  | Command                                      | When to use                        |
| ------------------------------------- | -------------------------------------------- | ---------------------------------- |
| Machine-readable repo snapshot (JSON) | `cargo xtask context`                        | Feed to agents or CI dashboards    |
| Daily orchestration workflow          | `cargo xtask daily-orchestration`            | CI-driven; use `--dry-run` locally |
| Host capability report                | `just doctor`                                | Verify cgroup/overlay/kernel state |
| Preflight tool check                  | `cargo xtask preflight`                      | Verify cargo, nextest, gh on PATH  |

### Benchmarks

| Task                                  | Command                            | When to use                              |
| ------------------------------------- | ---------------------------------- | ---------------------------------------- |
| Run criterion benchmarks              | `cargo xtask bench`                | Save results to bench/results/           |
| Sync VPS bench results locally        | `just bench-sync`                  | Pull jsonl from remote                   |
| Profile with samply/flamegraph        | `just flamegraph [suite]`          | macOS: samply; Linux: cargo-flamegraph   |
| AI bench analysis                     | `just bench-agent report`          | Summarise bench/results/ with AI         |

---

## scripts/ Directory

The `scripts/` directory contains AI agent tooling (council analysis,
AI-assisted code review, test generation) and operational helpers (VM setup,
daemon start, cgroup test harness). These are **not** part of the core
build/test pipeline. All Python scripts use `uv run` with PEP 723 inline
deps. See `just --list` for recipes that wrap them.
