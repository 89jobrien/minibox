import? 'sops.just'
default:
    @just --list

# ── Formatting ──────────────────────────────────────────────────────────────

fmt:
    crux run crux/dev/fmt.crux

fmt-check:
    crux run crux/dev/fmt_check.crux

# ── Linting ─────────────────────────────────────────────────────────────────

# Lint all crates (macOS-safe; miniboxd dispatches to macbox on macOS)
lint:
    crux run crux/dev/lint.crux

# ── Build ───────────────────────────────────────────────────────────────────

build:
    crux run crux/dev/build.crux

# Compile optimised binaries (macOS-safe; excludes miniboxd)
build-release:
    crux run crux/dev/build_release.crux

# Build the sandbox toolchain image and load into minibox.
build-sandbox:
    crux run crux/dev/build_sandbox.crux

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

# Install repo git hooks from .githooks/ — run once after cloning, and again
# whenever .githooks/* changes upstream (git does not auto-sync .git/hooks/).
install-hooks:
    crux run crux/dev/install_hooks.crux
    @echo "installed .git/hooks/pre-commit from .githooks/pre-commit"

# fmt-check + lint + build-release
pre-commit:
    crux run crux/dev/pre_commit.crux

# release build + nextest
prepush:
    crux run crux/dev/prepush.crux

# fmt-check + lint + test-unit
ci:
    just fmt-check
    just lint
    just test-unit

# Read-only local gate: fmt, check, clippy, borrow fixtures, docs lint
verify:
    crux run crux/dev/verify.crux

# ── Testing ─────────────────────────────────────────────────────────────────

# All unit + conformance tests (any platform)
test-unit:
    crux run crux/dev/test_unit.crux

# Property tests
test-property:
    crux run crux/dev/test_property.crux

# Adapter isolation tests (any platform)
test-adapters:
    crux run crux/dev/test_adapters.crux

# Fast parallel test runner via nextest
nextest:
    crux run crux/dev/nextest.crux

# HTML coverage report (opens at target/llvm-cov/html/index.html)
coverage:
    crux run crux/dev/coverage.crux
    @echo "coverage: target/llvm-cov/html/index.html"

# VZ isolation tests (macOS, requires VM image at ~/.minibox/vm/)
# Builds the test binary, codesigns it with the virtualization entitlement,
# then runs it directly (bypasses cargo test runner to preserve dispatch_main harness).
test-vz-isolation:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p macbox --features vz --test vz_isolation_tests
    TARGET_DIR="${CARGO_TARGET_DIR:-$(pwd)/target}"
    BIN=$(ls -t "$TARGET_DIR/debug/deps/vz_isolation_tests-"* | head -1)
    codesign --force --sign - --entitlements entitlements/vz-test.entitlements "$BIN"
    "$BIN"

# CLI subprocess integration tests (builds binary first, any platform)
test-cli-subprocess:
    crux run crux/dev/test_cli_subprocess.crux

# Cgroup integration tests (Linux, root)
test-integration:
    sudo -E cargo xtask run-cgroup-tests
    sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture
    sudo -E cargo test -p minibox --test native_adapter_isolation_tests -- --test-threads=1 --nocapture
    sudo -E cargo test -p minibox --test native_adapter_lifecycle_failure_tests -- --test-threads=1 --nocapture
    cargo test -p minibox --test gke_adapter_isolation_tests -- --test-threads=1 --nocapture

# Protocol e2e tests: any platform, no root, no cgroups
test-e2e:
    crux run crux/dev/test_e2e.crux

# System tests: full-stack daemon+CLI (Linux, root, cgroups v2 required)
test-system:
    crux run crux/dev/test_system.crux

# Sandbox contract tests (Linux, root, Docker Hub)
test-sandbox:
    crux run crux/dev/test_sandbox.crux

# Linux dogfood: build test image + load + run all tests inside container
test-linux:
    crux run crux/dev/test_linux.crux

# Run e2e suite on VPS (pulls latest main, runs as root, streams output)
test-e2e-vps:
    crux run crux/dev/test_e2e_vps.crux

# Clone an arbitrary branch fresh into a scratch dir on jobrien-vm (does not touch ~/minibox)
verify-vps branch:
    ssh -t jobrien-vm 'rm -rf ~/verify-{{branch}} && git clone --branch {{branch}} --single-branch git@github.com:89jobrien/minibox.git ~/verify-{{branch}}'

# Run the mount-immutability seccomp kernel-enforcement test on jobrien-vm (Linux-only, does not compile on macOS)
test-seccomp-vps branch="develop": (verify-vps branch)
    ssh -t jobrien-vm 'cd ~/verify-{{branch}} && ~/.cargo/bin/cargo test -p minibox --lib mount_seccomp -- --test-threads=1'

# Build release binaries on jobrien-vm for live smolvm pull/run verification (needs a real network path smolvm's guest can reach)
build-smolvm-vps branch="develop": (verify-vps branch)
    ssh -t jobrien-vm 'cd ~/verify-{{branch}} && ~/.cargo/bin/cargo build --release -p mbx -p miniboxd'

# Full pipeline: clean state -> doctor -> all tests -> clean state
test-all: nuke-test-state doctor test-unit test-integration test-system nuke-test-state

# ── Benchmarks ──────────────────────────────────────────────────────────────

bench:
    crux run crux/dev/bench.crux

# Run benches and compare against the tracked per-env baseline
bench-check:
    crux run crux/dev/bench_check.crux

# Run benches and save results as the new per-env baseline
bench-baseline:
    crux run crux/dev/bench_baseline.crux

# Machine-readable repo context snapshot (JSON to stdout)
context:
    crux run crux/dev/context.crux

# ── Daemon ──────────────────────────────────────────────────────────────────

doctor:
    @crux run crux/dev/doctor.crux || true
    @echo ""
    @echo "--- Host Capabilities Report ---"
    @cargo test -p minibox-core preflight::tests::test_format_report_does_not_panic -- --nocapture 2>&1 | grep -A 20 "Minibox Host Capabilities" || echo "Could not generate report (non-Linux host?)"

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
    crux run crux/dev/clean.crux

clean-artifacts:
    crux run crux/dev/clean_artifacts.crux

clean-test:
    find target/debug/deps -name '*_tests-*' -delete 2>/dev/null || true
    find target/debug/deps -name '*miniboxd-*' -delete 2>/dev/null || true

clean-stale days="7":
    find target/ -type f -mtime +{{days}} -delete 2>/dev/null || true
    find target/ -type d -empty -delete 2>/dev/null || true

nuke-test-state:
    crux run crux/dev/nuke_test_state.crux

# ── CI ──────────────────────────────────────────────────────────────────────

ci-watch *args:
    cargo xtask ci-watch {{args}}
