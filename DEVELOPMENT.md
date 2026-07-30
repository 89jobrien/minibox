# Development Guide

Canonical developer workflow for minibox. See `CLAUDE.md` for architecture details,
`docs/core/TESTING.mbx.md` for the full test strategy, and `docs/core/SUPPORT_TIERS.mbx.md` for crate and
adapter support-tier definitions.

## First Time Setup

```bash
# 1. Install git hooks (pre-commit, pre-push, commit-msg) — run once after cloning
just install-hooks

# 2. Verify your environment
cargo xtask doctor

# 3. Build everything
cargo build --release
```

## Daily Workflow

Three commands cover 95% of daily development:

```bash
cargo xtask pre-commit   # before every commit: staged fmt/clippy + config/docs checks
cargo xtask test unit    # run all unit + conformance tests (any platform)
cargo xtask prepush      # before every push: release build + release nextest + conformance
```

## Prerequisites

- Rust stable (edition 2024)
- [cargo-nextest](https://nexte.st/) for test filtering
- [just](https://github.com/casey/just) for convenience recipes
- Linux + root for integration/e2e tests (unit tests run on macOS)

## Optional

- [Nushell](https://www.nushell.sh/) — most `scripts/` helpers are written in `nu`

## Runner Hierarchy

Minibox has two task runners. They are complementary, not competing:

| Runner  | Role                              | When to use                  |
| ------- | --------------------------------- | ---------------------------- |
| `xtask` | CI gates, canonical test suites   | Always for CI-critical paths |
| `just`  | Wraps xtask + convenience recipes | Day-to-day development       |

**Rule of thumb:** if a GitHub Actions workflow calls it, the source of truth is
`cargo xtask <command>`. `just` recipes delegate to xtask where possible. `scripts/`
contains AI agent tooling and one-off helpers — not part of the core build/test pipeline.

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

- **Unset** (default): tries `smolvm`; falls back to `native` on Linux or
  `krun` on macOS if the `smolvm` binary is not on `PATH`.
- **Explicit** (`MINIBOX_ADAPTER=<name>`): uses the named adapter as-is, no fallback.

```bash
# Auto-select (smolvm → native on Linux, krun on macOS)
sudo ./target/release/miniboxd

# Pin to a specific adapter
sudo MINIBOX_ADAPTER=krun ./target/release/miniboxd
sudo MINIBOX_ADAPTER=native ./target/release/miniboxd   # Linux + root only

# Inspect compiled adapters
sudo ./target/release/mbx doctor
```

Do **not** set `MINIBOX_ADAPTER` inside start scripts or systemd units to hard-code an
adapter — this bypasses the smolvm fallback chain and will fail if the named adapter binary
is absent.

## Building

```bash
cargo build --release                # all crates
just build-release                   # optimised (macOS-safe)
just build-linux                     # static musl binary (auto-detects arch)
```

## Testing

### Unit tests (any platform)

```bash
cargo xtask test unit        # canonical
just test-unit               # equivalent shorthand
```

### Borrow-reasoning fixtures (any platform)

```bash
cargo xtask borrow-fixtures  # standalone must-pass/must-fail Rust borrow examples
```

The fixture suite checks standalone Rust examples with `rustc`: fixtures under
`xtask/fixtures/borrow/pass` are `must-pass`; fixtures under `xtask/fixtures/borrow/fail`
are `must-fail` with their declared `// expect: ...` diagnostic snippets.

### Integration tests (Linux + root)

```bash
just test-integration        # cgroup tests + native adapter isolation
```

`just test-integration` calls `cargo xtask run-cgroup-tests` and the integration test
binary directly with `sudo`. The `cargo xtask test integration` suite also exists,
but `just` is the documented local entrypoint because it handles the root-only flow.

### End-to-end tests (Linux + root)

```bash
cargo xtask test e2e         # canonical protocol e2e tests
just test-e2e-suite          # full daemon + CLI e2e suite
just test-e2e-vps            # run e2e suite on VPS via SSH
```

### Property-based tests (any platform)

```bash
cargo xtask test property
```

### Adapter and CLI tests (any platform)

```bash
just test-adapters           # Colima + handler adapter swap tests
just test-cli-subprocess     # CLI subprocess integration tests
```

### VM tests (macOS + smolvm)

```bash
cargo xtask test-linux       # build musl binary + run suite in VM
```

Requires `cargo xtask build-test-image` to have been run at least once.

### Coverage

```bash
just coverage                # HTML report at target/llvm-cov/html/
```

### Preflight / Doctor

```bash
cargo xtask doctor           # CANONICAL: tool checks + env + Linux system caps
mbx doctor                   # adapter diagnostics + delegates to cargo xtask doctor
```

`cargo xtask doctor` is the authoritative preflight command. It checks:

- Required tools on PATH: `cargo`, `just`, `rustup`, `cargo-nextest`
- Advisory tools: `gh`, `op` (warn, not fail)
- `CARGO_TARGET_DIR` env var (advisory)
- Linux-only: cgroups v2 unified hierarchy, overlay FS, kernel >= 5.0

`scripts/preflight.nu` is a lightweight SessionStart hook — it runs at shell startup to
surface obvious missing deps. It is not a substitute for `cargo xtask doctor`.

### Full pipeline

```bash
just test-all                # nuke state -> doctor -> unit + integration + e2e -> nuke
```

## Benchmarks

All criterion benchmarks live in `crates/minibox-bench`. Results are written to
`bench/results/` (gitignored); tracked per-env baselines live at
`bench/baseline.{local,selfhosted,hosted}.json`. The nightly CI bench job produces the
canonical numbers.

```bash
just bench                       # run benches, save to bench/results/
just bench-check                 # run benches, compare against the per-env baseline
just bench-baseline              # run benches, save results as the new baseline
cargo xtask bench --skip-bench   # re-parse existing criterion output without re-running
nu scripts/bench-agent.nu report # AI bench analysis
```

## CI Gates

Local validation should match CI. The commands that matter:

1. **Read-only local gate:** `cargo xtask verify`
2. **Before every commit:** `cargo xtask pre-commit`
3. **Before every push:** `cargo xtask prepush`

GitHub Actions (`pr.yml` + `merge.yml`) runs the same xtask commands plus
`cargo deny`, `cargo audit`, and `cargo machete` on the `next` and `staging` branches.

## Environment Variables

| Variable               | Purpose                                    | Default                            |
| ---------------------- | ------------------------------------------ | ---------------------------------- |
| `MINIBOX_ADAPTER`      | Adapter suite: native, gke, smolvm, krun   | auto: smolvm, fallback native/krun |
| `MINIBOX_DATA_DIR`     | Image/container storage                    | `/var/lib/minibox` (root)          |
| `MINIBOX_RUN_DIR`      | Socket/runtime directory                   | `/run/minibox`                     |
| `MINIBOX_SOCKET_PATH`  | Unix socket path                           | `$MINIBOX_RUN_DIR/miniboxd.sock`   |
| `MINIBOX_CGROUP_ROOT`  | Cgroup root for containers                 | systemd slice path                 |
| `MINIBOX_NETWORK_MODE` | Network mode: none, bridge                 | `none`                             |
| `RUST_LOG`             | Tracing verbosity (debug, info, warn, etc) | unset                              |

## Cleanup

```bash
cargo xtask nuke-test-state  # kill orphans, unmount overlays, clean cgroups
cargo xtask clean-artifacts  # remove non-critical build outputs
just clean-stale [days]      # remove files older than N days (default: 7)
just clean                   # full cargo clean (nuclear option)
```

## Command Reference

All commands exist in either `cargo xtask` or `just`. Commands marked _(Linux/root)_
require a Linux host with root privileges.

### Quality Gates

| Task                         | Command                  | Notes                                         |
| ---------------------------- | ------------------------ | --------------------------------------------- |
| Before every commit          | `cargo xtask pre-commit` | staged fmt/clippy + config/docs checks        |
| Before every push            | `cargo xtask prepush`    | release build + release nextest + conformance |
| Read-only local verification | `cargo xtask verify`     | fmt + clippy + borrow fixtures + docs         |
| Auto-fix formatting/clippy   | `cargo xtask fix`        | Mutates files; review diff after              |
| Lint only (no build)         | `cargo xtask lint`       | fmt-check + clippy (CI lint gate)             |

### Testing

| Task                       | Command                         | Platform / Notes                         |
| -------------------------- | ------------------------------- | ---------------------------------------- |
| Unit + conformance tests   | `cargo xtask test unit`         | Any platform; no root required           |
| Property-based tests       | `cargo xtask test property`     | Any platform; proptest suite             |
| Borrow-reasoning fixtures  | `cargo xtask borrow-fixtures`   | Standalone rustc must-pass/must-fail     |
| Protocol e2e tests         | `cargo xtask test e2e`          | Any platform; no root required           |
| Cgroup integration tests   | `just test-integration`         | Linux + root; cgroup v2                  |
| Full-stack system tests    | `cargo xtask test system-suite` | Linux + root; daemon + CLI               |
| Sandbox contract tests     | `cargo xtask test sandbox`      | Linux + root; requires Docker Hub        |
| Adapter isolation tests    | `just test-adapters`            | Any platform                             |
| CLI subprocess tests       | `just test-cli-subprocess`      | Any platform; builds mbx first           |
| Linux dogfood (build + VM) | `cargo xtask test-linux`        | macOS + smolvm; runs suite in container  |
| Full pipeline              | `just test-all`                 | Linux + root; nuke -> all tests -> nuke  |
| Remote VPS e2e             | `just test-e2e-vps`             | Runs `test system-suite` on VPS over SSH |
| HTML coverage report       | `just coverage`                 | Any platform; opens target/llvm-cov/     |

### Codebase Integrity Checks

| Task                               | Command                                     | When to use                         |
| ---------------------------------- | ------------------------------------------- | ----------------------------------- |
| Detect stale crate/binary names    | `cargo xtask check-stale-names`             | After renames; CI gate              |
| Verify protocol contract hashes    | `cargo xtask check-protocol-drift`          | After protocol.rs changes           |
| Update protocol hash baseline      | `cargo xtask check-protocol-drift --update` | After intentional protocol changes  |
| Verify HandlerDependencies count   | `cargo xtask check-protocol-sites`          | After adding/removing handler sites |
| Scan for `.unwrap()` in production | `cargo xtask check-no-unwrap`               | Advisory; use `--strict` to fail    |
| Verify adapter test coverage       | `cargo xtask check-adapter-coverage`        | After adding a new adapter          |
| Check for tracked generated files  | `cargo xtask check-repo-clean`              | Before PRs                          |
| Lint docs frontmatter              | `cargo xtask docs lint`                     | After editing docs/core/            |

### Build

| Task                       | Command                        | Notes                            |
| -------------------------- | ------------------------------ | -------------------------------- |
| Debug build (all crates)   | `cargo build`                  | Fast iteration                   |
| Release build (all crates) | `cargo build --release`        | Pre-deployment check             |
| Optimised macOS-safe build | `just build-release`           | macOS dev                        |
| Static musl Linux binary   | `just build-linux`             | Cross-compile for VPS deployment |
| Build + load test image    | `cargo xtask build-test-image` | Required before `test-linux`     |

### Cleanup

| Task                           | Command                       | Notes                                 |
| ------------------------------ | ----------------------------- | ------------------------------------- |
| Kill orphans, unmount overlays | `cargo xtask nuke-test-state` | After failed tests leave state behind |
| Remove non-critical outputs    | `cargo xtask clean-artifacts` | After a release build                 |
| Full cargo clean               | `just clean`                  | Nuclear option                        |
| Remove stale build artifacts   | `just clean-stale [days]`     | Reclaim disk (default: 7 days)        |

### Repo Context & Orchestration

| Task                                 | Command                                | Notes                              |
| ------------------------------------ | -------------------------------------- | ---------------------------------- |
| Machine-readable repo snapshot       | `cargo xtask context`                  | Feed to agents or CI dashboards    |
| Daily orchestration workflow         | `cargo xtask daily-orchestration`      | CI-driven; use `--dry-run` locally |
| Host capability report               | `cargo xtask doctor`                   | Verify cgroup/overlay/kernel state |
| Preflight tool check                 | `cargo xtask preflight`                | Verify cargo, nextest, gh on PATH  |
| Watch latest CI run (current branch) | `cargo xtask ci-watch`                 | Job-level detail + live tail       |
| Watch CI run on a specific branch    | `cargo xtask ci-watch --branch <name>` | Monitor another branch             |

### Benchmarks

| Task                     | Command                            | Notes                                 |
| ------------------------ | ---------------------------------- | ------------------------------------- |
| Run criterion benchmarks | `just bench`                       | Targets in crates/minibox-bench       |
| Check against baseline   | `just bench-check`                 | Compares vs bench/baseline.{env}.json |
| Save new baseline        | `just bench-baseline`              | Writes bench/baseline.{env}.json      |
| AI bench analysis        | `nu scripts/bench-agent.nu report` | Summarise bench/results/ with AI      |

---

## scripts/ Directory

The `scripts/` directory contains AI agent tooling and operational helpers. These are
**not** part of the core build/test pipeline. See `just --list` for recipes that wrap them.

| Script                    | Purpose                                                  |
| ------------------------- | -------------------------------------------------------- |
| `ci-watch.nu`             | Watch latest GHA run with job-level detail (wraps xtask) |
| `preflight.nu`            | SessionStart hook — lightweight tool/env check           |
| `promote.nu`              | Cascade-merge through develop → next → staging → main    |
| `sync-check.nu`           | Fetch + rebase check before push                         |
| `ai-review.nu`            | AI-assisted code review of staged diff                   |
| `council.nu`              | Multi-model council analysis                             |
| `standup.nu`              | Generate standup summary from recent commits             |
| `diagnose.nu`             | Container/daemon diagnostics                             |
| `start-daemon.nu`         | Start miniboxd with adapter selection                    |
| `build-test-image.nu`     | Build test OCI image for VM tests                        |
| `vm-setup.nu`             | Provision VM environment                                 |
| `dashboard.nu`            | Render local metrics dashboard                           |
| `collect-metrics.nu`      | Aggregate crate/test/line counts (JSON)                  |
| `gen-tests.nu`            | AI-assisted test scaffolding                             |
| `bench-agent.nu`          | AI bench result analysis                                 |
| `meta-agent.nu`           | Orchestration meta-agent                                 |
| `commit-msg.nu`           | Commit message hook helper                               |
| `install-hooks.nu`        | Install git hooks (pre-commit, pre-push, commit-msg)     |
| `check-protocol-sites.nu` | Protocol site audit (wraps xtask)                        |
| `trace-lima.nu`           | Trace Lima VM activity                                   |
| `parse-geiger.nu`         | Parse cargo-geiger unsafe usage output                   |
