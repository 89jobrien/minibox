default:
    @just --list

# ── Formatting ──────────────────────────────────────────────────────────────

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

# ── Linting ─────────────────────────────────────────────────────────────────

# Lint all crates (macOS-safe; miniboxd dispatches to macbox on macOS)
lint:
    cargo clippy -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -- -D warnings

# ── Build ────────────────────────────────────────────────────────────────────

# Compile optimised binaries (macOS-safe; excludes miniboxd)
build-release:
    cargo build --release -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p minibox-bench

build:
    cargo build --release

# Build static Linux musl binaries matching the host architecture.
# Output: target/<arch>-unknown-linux-musl/release/{miniboxd,minibox}
build-linux:
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -m)" in
        arm64|aarch64) MUSL_TARGET="aarch64-unknown-linux-musl" ;;
        x86_64|amd64)  MUSL_TARGET="x86_64-unknown-linux-musl" ;;
        *) echo "error: unsupported arch $(uname -m)"; exit 1 ;;
    esac
    rustup target add "$MUSL_TARGET"
    RUSTFLAGS="-C target-feature=+crt-static" \
        cargo build --release --target "$MUSL_TARGET" \
        -p miniboxd -p minibox-cli

# ── Gates ────────────────────────────────────────────────────────────────────

# fmt-check + lint + build-release
pre-commit:
    cargo xtask pre-commit

# nextest + coverage + flamegraph
prepush:
    cargo xtask prepush

# fmt-check + lint + test-unit
ci:
    cargo fmt --all --check
    cargo clippy -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -- -D warnings
    just test-unit

# ── Testing ──────────────────────────────────────────────────────────────────

# All unit + conformance tests (any platform)
test-unit:
    cargo xtask test-unit

# Adapter isolation tests (any platform)
test-adapters:
    cargo test -p linuxbox --test adapter_colima_tests
    cargo test -p daemonbox --test handler_adapter_swap_tests

# Fast parallel test runner via nextest
nextest:
    cargo nextest run --release -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox

# HTML coverage report (opens at target/llvm-cov/html/index.html)
coverage:
    cargo llvm-cov nextest -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox --html
    @echo "coverage: target/llvm-cov/html/index.html"

# CLI subprocess integration tests (builds binary first, any platform)
test-cli-subprocess:
    cargo build -p minibox-cli
    MINIBOX_TEST_BIN_DIR={{justfile_directory()}}/target/debug \
        cargo test -p minibox-cli --features subprocess-tests --test cli_subprocess

# Cgroup integration tests (Linux, root)
test-integration:
    sudo -E cargo test -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture
    sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture

# Lifecycle e2e (Linux, root, Docker Hub)
test-e2e:
    sudo -E cargo test -p miniboxd --test integration_tests -- --ignored test_complete_container_lifecycle

# Daemon+CLI e2e tests (Linux, root)
test-e2e-suite:
    cargo xtask test-e2e-suite

# Sandbox contract tests (Linux, root, Docker Hub)
test-sandbox:
    cargo xtask test-sandbox

# Full pipeline: clean state → doctor → all tests → clean state
test-all: nuke-test-state doctor test-unit test-integration test-e2e nuke-test-state

# ── Benchmarks ───────────────────────────────────────────────────────────────

bench:
    cargo xtask bench

# Profile bench binary with samply (macOS) or cargo-flamegraph (Linux)
# Usage: just flamegraph [suite]   (default suite: codec)
flamegraph suite="codec":
    cargo xtask flamegraph --suite {{suite}}

# AI bench analysis (subcommands: report, compare, regress, cleanup, trigger)
bench-agent *args:
    #!/usr/bin/env bash
    uv run scripts/bench-agent.py "$@"

# ── Daemon ───────────────────────────────────────────────────────────────────

doctor:
    @cargo test -p linuxbox preflight::tests -- --nocapture 2>&1 || true
    @echo ""
    @echo "--- Host Capabilities Report ---"
    @cargo test -p linuxbox preflight::tests::test_format_report_does_not_panic -- --nocapture 2>&1 | grep -A 20 "Minibox Host Capabilities" || echo "Could not generate report (non-Linux host?)"

# Trace miniboxd with uftrace.
# macOS: cross-compiles Linux binary, runs it inside minibox via Colima.
# Linux: runs natively (requires root + apt install uftrace).
# After run: uftrace graph -d <trace-dir>
trace:
    #!/usr/bin/env bash
    set -euo pipefail

    TRACE_DIR="traces/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$TRACE_DIR"

    if [[ "$(uname -s)" == "Darwin" ]]; then
        # The Colima adapter does not stream container stdout back through minibox
        # (spawn_process returns output_reader: None). uftrace is a Linux tool anyway.
        # Run the trace directly inside the Lima VM via limactl shell, bypassing minibox.
        echo "trace: building Linux musl binary..."
        just build-linux

        TARGET_DIR="${CARGO_TARGET_DIR:-$(pwd)/target}"
        case "$(uname -m)" in
            arm64|aarch64) MUSL_TARGET="aarch64-unknown-linux-musl" ;;
            x86_64|amd64)  MUSL_TARGET="x86_64-unknown-linux-musl" ;;
            *) echo "error: unsupported arch"; exit 1 ;;
        esac
        BINARY_DIR="${TARGET_DIR}/${MUSL_TARGET}/release"
        ABS_TRACE="$(pwd)/$TRACE_DIR"

        # Lima mounts /tmp and /Users into the VM — both paths are accessible.
        echo "trace: running uftrace inside Colima VM..."
        colima ssh -- bash "$(pwd)/scripts/trace-lima.sh" "$BINARY_DIR" "$ABS_TRACE"

        echo ""
        echo "── uftrace report (top 20 by total time) ──────────────────────────────"
        colima ssh -- uftrace report -d "${ABS_TRACE}" --sort=total 2>/dev/null | head -25 || echo "(no trace data)"
    else
        [[ "$(uname -s)" == "Linux" ]] || { echo "error: unsupported platform"; exit 1; }
        command -v uftrace >/dev/null 2>&1 || { echo "error: apt install uftrace"; exit 1; }
        [[ "$(id -u)" -eq 0 ]] || { echo "error: sudo just trace"; exit 1; }

        echo "trace: building native release binary..."
        cargo build --release -p miniboxd -p minibox-cli

        echo "trace: recording to $TRACE_DIR ..."
        uftrace record -P . --no-libcall -d "$TRACE_DIR" ./target/release/miniboxd &
        DAEMON_PID=$!

        for i in $(seq 1 10); do
            [[ -S /run/minibox/miniboxd.sock ]] && break
            sleep 0.5
        done
        [[ -S /run/minibox/miniboxd.sock ]] || { echo "error: daemon socket did not appear"; kill "$DAEMON_PID" 2>/dev/null; exit 1; }

        echo "trace: smoke — pull alpine..."
        ./target/release/minibox pull alpine || true
        echo "trace: smoke — run echo..."
        ./target/release/minibox run alpine -- /bin/echo "uftrace smoke" || true

        echo "trace: stopping daemon..."
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true

        echo ""
        echo "── uftrace report (top 20 by total time) ──────────────────────────────"
        uftrace report -d "$TRACE_DIR" --sort=total 2>/dev/null | head -25 || echo "(no trace data)"
    fi

    echo ""
    echo "trace: data saved to $TRACE_DIR"
    echo "trace: call graph      → uftrace graph -d $TRACE_DIR"
    echo "trace: chrome devtools → uftrace dump -d $TRACE_DIR --chrome > $TRACE_DIR/trace.json"

# ── AI Agents ────────────────────────────────────────────────────────────────

# Meta-agent: designs + spawns parallel agents from user intent (e.g. just meta-agent "audit the overlay mount code")
meta-agent task:
    uv run scripts/meta-agent.py {{ quote(task) }}

# Multi-role council analysis of current branch (core: 3 roles, extensive: 5 roles)
council base="main" mode="core":
    uv run scripts/council.py --base {{ quote(base) }} --mode {{ quote(mode) }}

# AI code review vs main (security + correctness focused)
ai-review base="main":
    uv run scripts/ai-review.py --base {{ quote(base) }}

# Generate unit tests for a domain trait adapter (e.g. just gen-tests BridgeNetworking)
gen-tests trait:
    uv run scripts/gen-tests.py {{ quote(trait) }}

# Diagnose latest container failure from logs + cgroup state
diagnose *args:
    #!/usr/bin/env bash
    uv run scripts/diagnose.py "$@"

# Fetch, check sync vs origin/main, auto-resolve obvious conflicts (prompts if unsure)
sync-check:
    uv run scripts/sync-check.py

# ── Git ──────────────────────────────────────────────────────────────────────

# Sync-check then push + clean non-critical artifacts
push *args:
    uv run scripts/sync-check.py
    git push {{args}}
    cargo xtask clean-artifacts

# Fetch + rebase onto origin/main
pull:
    git fetch origin
    git rebase origin/main

# Stage all + commit (triggers pre-commit hook)
commit msg:
    git add -A
    git commit -m "{{msg}}"

# Generate a commit message from staged changes (use -a to stage all, -c to commit)
commit-msg *args:
    #!/usr/bin/env bash
    uv run scripts/commit-msg.py "$@"

# ── Cleanup ───────────────────────────────────────────────────────────────────

clean-artifacts:
    cargo xtask clean-artifacts

clean:
    cargo clean

clean-test:
    find target/debug/deps -name '*_tests-*' -delete 2>/dev/null || true
    find target/debug/deps -name '*miniboxd-*' -delete 2>/dev/null || true

clean-stale days="7":
    find target/ -type f -mtime +{{days}} -delete 2>/dev/null || true
    find target/ -type d -empty -delete 2>/dev/null || true

nuke-test-state:
    cargo xtask nuke-test-state

metrics-report:
    uv run python scripts/collect_metrics.py --reports-dir artifacts/reports

# ── Agentbox (Go) ─────────────────────────────────────────────────────

# Build all agentbox binaries
agentbox-build:
    cd agentbox && go build ./cmd/agentbox/ && go build ./cmd/mbx-commit-msg/

# Run agentbox tests
agentbox-test:
    cd agentbox && go test ./... -v

# Run council analysis (Go)
agentbox-council *ARGS:
    cd agentbox && op run --account=my.1password.com --env-file=$HOME/.secrets -- go run ./cmd/agentbox/ council {{ARGS}}

# Run meta-agent (Go)
agentbox-meta-agent *ARGS:
    cd agentbox && op run --account=my.1password.com --env-file=$HOME/.secrets -- go run ./cmd/agentbox/ meta-agent {{ARGS}}

# Generate commit message (Go)
agentbox-commit-msg *ARGS:
    cd agentbox && go run ./cmd/mbx-commit-msg/ {{ARGS}}
