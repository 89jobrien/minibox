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
    cargo clippy -p minibox -p minibox-core -p minibox-macros -p minibox-crux-plugin -p mbx -p macbox -p miniboxd -- -D warnings

# ── Build ───────────────────────────────────────────────────────────────────

build:
    cargo build --release

# Compile optimised binaries (macOS-safe; excludes miniboxd)
build-release:
    cargo build --release -p minibox -p minibox-core -p minibox-macros -p minibox-crux-plugin -p mbx -p miniboxd

# Build the sandbox toolchain image and load into minibox.
build-sandbox:
    bash images/sandbox/build.sh

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
        -p miniboxd -p mbx

# ── Gates ───────────────────────────────────────────────────────────────────

# fmt-check + lint + build-release
pre-commit:
    cargo xtask pre-commit

# release build + nextest
prepush:
    cargo xtask prepush

# fmt-check + lint + test-unit
ci:
    cargo fmt --all --check
    just lint
    just test-unit

# Read-only local gate: fmt, check, clippy, borrow fixtures, docs lint
verify:
    cargo xtask verify

# ── Testing ─────────────────────────────────────────────────────────────────

# All unit + conformance tests (any platform)
test-unit:
    cargo xtask test-unit

# Property tests
test-property:
    cargo xtask test-property

# Adapter isolation tests (any platform)
test-adapters:
    cargo test -p minibox --test adapter_colima_tests
    cargo test -p minibox --test daemon_handler_adapter_swap_tests

# Fast parallel test runner via nextest
nextest:
    cargo nextest run --release -p minibox -p minibox-core -p minibox-macros -p minibox-crux-plugin -p mbx -p miniboxd

# HTML coverage report (opens at target/llvm-cov/html/index.html)
coverage:
    cargo llvm-cov nextest -p minibox -p minibox-core -p minibox-macros -p minibox-crux-plugin -p mbx -p miniboxd --html
    @echo "coverage: target/llvm-cov/html/index.html"

# CLI subprocess integration tests (builds binary first, any platform)
test-cli-subprocess:
    cargo build -p mbx
    MINIBOX_TEST_BIN_DIR={{justfile_directory()}}/target/debug \
        cargo test -p mbx --features subprocess-tests --test cli_subprocess

# Cgroup integration tests (Linux, root)
test-integration:
    sudo -E cargo xtask run-cgroup-tests
    sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture
    sudo -E cargo test -p minibox --test native_adapter_isolation_tests -- --test-threads=1 --nocapture
    cargo test -p minibox --test gke_adapter_isolation_tests -- --test-threads=1 --nocapture

# Protocol e2e tests: any platform, no root, no cgroups
test-e2e:
    cargo xtask test-e2e

# System tests: full-stack daemon+CLI (Linux, root, cgroups v2 required)
test-system:
    cargo xtask test-system-suite

# Sandbox contract tests (Linux, root, Docker Hub)
test-sandbox:
    cargo xtask test-sandbox

# Linux dogfood: build test image + load + run all tests inside container
test-linux:
    cargo xtask test-linux

# Run e2e suite on VPS (pulls latest main, runs as root, streams output)
test-e2e-vps:
    ssh -t jobrien-vm 'cd ~/minibox && git pull && sudo -E env PATH="/home/dev/.cargo/bin:$PATH" cargo xtask test-system-suite'

# Full pipeline: clean state -> doctor -> all tests -> clean state
test-all: nuke-test-state doctor test-unit test-integration test-system nuke-test-state

# ── Benchmarks ──────────────────────────────────────────────────────────────

bench:
    cargo xtask bench

# Run benches and compare against the tracked per-env baseline
bench-check:
    cargo xtask bench --check

# Run benches and save results as the new per-env baseline
bench-baseline:
    cargo xtask bench --save-baseline

# Machine-readable repo context snapshot (JSON to stdout)
context:
    cargo xtask context

# ── Daemon ──────────────────────────────────────────────────────────────────

doctor:
    @cargo test -p minibox preflight::tests -- --nocapture 2>&1 || true
    @echo ""
    @echo "--- Host Capabilities Report ---"
    @cargo test -p minibox preflight::tests::test_format_report_does_not_panic -- --nocapture 2>&1 | grep -A 20 "Minibox Host Capabilities" || echo "Could not generate report (non-Linux host?)"

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

        echo "trace: running uftrace inside Colima VM..."
        colima ssh -- bash "$(pwd)/scripts/trace-lima.sh" "$BINARY_DIR" "$ABS_TRACE"

        echo ""
        echo "-- uftrace report (top 20 by total time) ------"
        colima ssh -- uftrace report -d "${ABS_TRACE}" --sort=total 2>/dev/null | head -25 || echo "(no trace data)"
    else
        [[ "$(uname -s)" == "Linux" ]] || { echo "error: unsupported platform"; exit 1; }
        command -v uftrace >/dev/null 2>&1 || { echo "error: apt install uftrace"; exit 1; }
        [[ "$(id -u)" -eq 0 ]] || { echo "error: sudo just trace"; exit 1; }

        echo "trace: building native release binary..."
        cargo build --release -p miniboxd -p mbx

        echo "trace: recording to $TRACE_DIR ..."
        uftrace record -P . --no-libcall -d "$TRACE_DIR" ./target/release/miniboxd &
        DAEMON_PID=$!

        for i in $(seq 1 10); do
            [[ -S /run/minibox/miniboxd.sock ]] && break
            sleep 0.5
        done
        [[ -S /run/minibox/miniboxd.sock ]] || { echo "error: daemon socket did not appear"; kill "$DAEMON_PID" 2>/dev/null; exit 1; }

        echo "trace: smoke -- pull alpine..."
        ./target/release/mbx pull alpine || true
        echo "trace: smoke -- run echo..."
        ./target/release/mbx run alpine -- /bin/echo "uftrace smoke" || true

        echo "trace: stopping daemon..."
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true

        echo ""
        echo "-- uftrace report (top 20 by total time) ------"
        uftrace report -d "$TRACE_DIR" --sort=total 2>/dev/null | head -25 || echo "(no trace data)"
    fi

    echo ""
    echo "trace: data saved to $TRACE_DIR"
    echo "trace: call graph      -> uftrace graph -d $TRACE_DIR"
    echo "trace: chrome devtools -> uftrace dump -d $TRACE_DIR --chrome > $TRACE_DIR/trace.json"

# ── Cleanup ─────────────────────────────────────────────────────────────────

clean:
    cargo clean

clean-artifacts:
    cargo xtask clean-artifacts

clean-test:
    find target/debug/deps -name '*_tests-*' -delete 2>/dev/null || true
    find target/debug/deps -name '*miniboxd-*' -delete 2>/dev/null || true

clean-stale days="7":
    find target/ -type f -mtime +{{days}} -delete 2>/dev/null || true
    find target/ -type d -empty -delete 2>/dev/null || true

nuke-test-state:
    cargo xtask nuke-test-state

# ── CI ──────────────────────────────────────────────────────────────────────

ci-watch *args:
    cargo xtask ci-watch {{args}}
