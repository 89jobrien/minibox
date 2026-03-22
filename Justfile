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
    cargo clippy -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -- -D warnings

# ── Build ────────────────────────────────────────────────────────────────────

# Compile optimised binaries (macOS-safe; excludes miniboxd)
build-release:
    cargo build --release -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -p minibox-bench

build:
    cargo build --release

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
    cargo clippy -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -- -D warnings
    just test-unit

# ── Testing ──────────────────────────────────────────────────────────────────

# All unit + conformance tests (any platform)
test-unit:
    cargo xtask test-unit

# Adapter isolation tests (any platform)
test-adapters:
    cargo test -p minibox-lib --test adapter_colima_tests
    cargo test -p daemonbox --test handler_adapter_swap_tests

# Fast parallel test runner via nextest
nextest:
    cargo nextest run --release -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox

# HTML coverage report (opens at target/llvm-cov/html/index.html)
coverage:
    cargo llvm-cov nextest -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox --html
    @echo "coverage: target/llvm-cov/html/index.html"

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

# Full pipeline: clean state → doctor → all tests → clean state
test-all: nuke-test-state doctor test-unit test-integration test-e2e nuke-test-state

# ── Benchmarks ───────────────────────────────────────────────────────────────

bench:
    cargo xtask bench

# ── Daemon ───────────────────────────────────────────────────────────────────

doctor:
    @cargo test -p minibox-lib preflight::tests -- --nocapture 2>&1 || true
    @echo ""
    @echo "--- Host Capabilities Report ---"
    @cargo test -p minibox-lib preflight::tests::test_format_report_does_not_panic -- --nocapture 2>&1 | grep -A 20 "Minibox Host Capabilities" || echo "Could not generate report (non-Linux host?)"

# ── AI Agents ────────────────────────────────────────────────────────────────

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
