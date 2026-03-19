default:
    @just --list

# Format all crates
fmt:
    cargo fmt --all

# Check formatting without applying
fmt-check:
    cargo fmt --all -- --check

# Lint cross-platform crates (macOS-safe; excludes miniboxd which has compile_error! on macOS)
lint:
    cargo clippy -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -- -D warnings

# Pre-commit: format check + lint + unit tests (macOS-safe)
pre-commit: fmt-check lint test-unit
    @echo "Pre-commit checks passed"

# CI gate (same as pre-commit; used in workflows)
ci: fmt-check lint test-unit
    @echo "CI checks passed"

# Preflight capability check
doctor:
    @cargo test -p minibox-lib preflight::tests -- --nocapture 2>&1 || true
    @echo ""
    @echo "--- Host Capabilities Report ---"
    @cargo test -p minibox-lib preflight::tests::test_format_report_does_not_panic -- --nocapture 2>&1 | grep -A 20 "Minibox Host Capabilities" || echo "Could not generate report (non-Linux host?)"

# Build release binaries
build:
    cargo build --release

fix-socket:
    cargo build --release -p miniboxd
    bash ops/fix-minibox-socket.sh

smoke:
    @bash -lc 'set -euo pipefail; sudo ./target/release/miniboxd & pid=$!; sleep 1; sudo ./target/release/minibox ps; sudo kill $pid; wait $pid || true'

# Unit tests (mock-based, any platform)
test-unit:
    cargo test -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox --lib
    cargo test -p daemonbox --test handler_tests
    cargo test -p daemonbox --test conformance_tests

# Fast parallel test runner via nextest (any platform; reuses pre-commit build artifacts)
nextest:
    cargo nextest run -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox

# Cgroup integration tests (Linux, root)
test-integration:
    sudo -E cargo test -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture
    sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture

# Lifecycle e2e (Linux, root, Docker Hub)
test-e2e:
    sudo -E cargo test -p miniboxd --test integration_tests -- --ignored test_complete_container_lifecycle

# Daemon+CLI e2e tests (Linux, root)
# Build as current user, run compiled test binary under sudo to avoid root-owned target/ files.
test-e2e-suite:
    cargo build --release
    cargo test -p miniboxd --test e2e_tests --release --no-run --message-format=json 2>/dev/null | jq -r 'select(.executable) | .executable' > /tmp/minibox-e2e-bin
    sudo -E MINIBOX_TEST_BIN_DIR={{justfile_directory()}}/target/release $(cat /tmp/minibox-e2e-bin) --test-threads=1 --nocapture

# Full pipeline: clean state → doctor → all tests → clean state
test-all: nuke-test-state doctor test-unit test-integration test-e2e nuke-test-state

# Stage all + commit (triggers pre-commit hook); use single quotes for multi-line messages
commit msg:
    git add -A
    git commit -m "{{msg}}"

# Push and clean non-critical artifacts on success
# Preserves: incremental cache, registry (keeps future compiles fast)
# Removes: debug/release executables, test binaries, .dSYM bundles
push *args:
    git push {{args}}
    just clean-artifacts

# Remove non-critical build outputs (preserves incremental cache and registry)
clean-artifacts:
    find target/debug -maxdepth 1 -type f -delete 2>/dev/null || true
    find target/release -maxdepth 1 -type f -delete 2>/dev/null || true
    find target -type d -name '*.dSYM' -exec rm -rf {} + 2>/dev/null || true
    find target/debug/deps -maxdepth 1 -type f ! -name '*.d' -delete 2>/dev/null || true
    find target/release/deps -maxdepth 1 -type f ! -name '*.d' -delete 2>/dev/null || true
    @echo "artifacts cleaned"

# Remove all build artifacts
clean:
    cargo clean

# Remove only test-related build artifacts
clean-test:
    find target/debug/deps -name '*_tests-*' -delete 2>/dev/null || true
    find target/debug/deps -name '*miniboxd-*' -delete 2>/dev/null || true

# Remove target/ artifacts older than N days (default 7)
clean-stale days="7":
    find target/ -type f -mtime +{{days}} -delete 2>/dev/null || true
    find target/ -type d -empty -delete 2>/dev/null || true

# Kill orphan processes, unmount overlays, remove test cgroups, clean temp dirs
nuke-test-state:
    #!/usr/bin/env bash
    set -euo pipefail
    pkill -f 'miniboxd.*minibox-test' 2>/dev/null || true
    mount | grep 'minibox-test' | awk '{print $3}' | xargs -r umount 2>/dev/null || true
    systemctl list-units --type=scope --no-legend 2>/dev/null | grep minibox-test | awk '{print $1}' | xargs -r systemctl stop 2>/dev/null || true
    find /sys/fs/cgroup -name 'minibox-test-*' -type d -exec rmdir {} \; 2>/dev/null || true
    rm -rf /tmp/minibox-test-* 2>/dev/null || true
    echo "test state cleaned"

bench:
    cargo build -p minibox-bench
    ./target/debug/minibox-bench --dry-run
    ./target/debug/minibox-bench

metrics-report:
    uv run python scripts/collect_metrics.py --reports-dir artifacts/reports
