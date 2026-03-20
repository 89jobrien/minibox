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

# ── Profiling ────────────────────────────────────────────────────────────────

bench:
    cargo xtask bench

flamegraph:
    samply record ./target/release/minibox-bench

# ── Daemon ───────────────────────────────────────────────────────────────────

doctor:
    @cargo test -p minibox-lib preflight::tests -- --nocapture 2>&1 || true
    @echo ""
    @echo "--- Host Capabilities Report ---"
    @cargo test -p minibox-lib preflight::tests::test_format_report_does_not_panic -- --nocapture 2>&1 | grep -A 20 "Minibox Host Capabilities" || echo "Could not generate report (non-Linux host?)"

fix-socket:
    cargo build --release -p miniboxd
    bash ops/fix-minibox-socket.sh

smoke:
    @bash -lc 'set -euo pipefail; sudo ./target/release/miniboxd & pid=$!; sleep 1; sudo ./target/release/minibox ps; sudo kill $pid; wait $pid || true'

# ── Demo ─────────────────────────────────────────────────────────────────────

# Pull an OCI image from Docker Hub — any platform, no daemon required
demo-pull image="alpine":
    cargo run --release --example pull -p minibox-lib -- {{image}}

# Pull 15 real images, run a probe in each, show timings + metrics, clean up
demo-showcase:
    #!/usr/bin/env bash
    set -euo pipefail
    C='\033[36m'; B='\033[1m'; G='\033[32m'; BL='\033[34m'; R='\033[0m'
    step() { printf "  ${B}${BL}▸${R}  ${B}%s${R}\n" "$1"; }
    ok()   { printf "  ${G}✓${R}  %s\n" "$1"; }
    spin() {
        local frames=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏') i=0 msg="$1"
        while true; do printf "\r     ${C}${frames[$i]}${R}  %s" "$msg"; i=$(( (i+1)%10 )); sleep 0.08; done
    }

    step "building"
    if [[ "$(uname)" == "Linux" ]]; then
        cargo build --release -p minibox-lib -p minibox-cli -p miniboxd --example showcase \
            2>&1 | grep -E "^(Compiling|Finished|error)" | tail -1
    else
        cargo build --release -p minibox-lib --example showcase \
            2>&1 | grep -E "^(Compiling|Finished|error)" | tail -1
    fi
    ok "binaries ready"; echo ""

    if [[ "$(uname)" == "Linux" ]]; then
        step "starting daemon"
        sudo ./target/release/miniboxd &>/dev/null &
        DAEMON_PID=$!
        trap 'sudo kill $DAEMON_PID 2>/dev/null; wait $DAEMON_PID 2>/dev/null || true' EXIT
        spin "waiting for daemon..." & SPIN=$!
        until sudo bash -c '[ -S /run/minibox/miniboxd.sock ]' 2>/dev/null; do sleep 0.05; done
        kill $SPIN 2>/dev/null; wait $SPIN 2>/dev/null || true
        printf "\r  ${G}✓${R}  daemon ready                        \n\n"
        sudo MINIBOX_DATA_DIR=/var/lib/minibox \
            ./target/release/examples/showcase \
            --run ./target/release/minibox --sudo-run --cleanup
    else
        ./target/release/examples/showcase --cleanup
    fi

# Hello World: build, start daemon, pull alpine, stream a greeting, stop daemon (Linux, root)
demo-hello:
    #!/usr/bin/env bash
    set -euo pipefail
    C='\033[36m'; B='\033[1m'; G='\033[32m'; BL='\033[34m'; R='\033[0m'
    hdr() { printf "\n${B}${C}  ╭─────────────────────────────────────────╮\n  │  %-39s  │\n  ╰─────────────────────────────────────────╯${R}\n\n" "$1"; }
    step() { printf "  ${B}${BL}▸${R}  ${B}%s${R}\n" "$1"; }
    ok()   { printf "  ${G}✓${R}  %s\n" "$1"; }
    spin() {
        local frames=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏') i=0 msg="$1"
        while true; do printf "\r     ${C}${frames[$i]}${R}  %s" "$msg"; i=$(( (i+1)%10 )); sleep 0.08; done
    }

    if [[ "$(uname)" != "Linux" ]]; then
        printf "  ${C}demo-hello requires Linux + root — try: just demo-pull${R}\n"; exit 1
    fi

    hdr "minibox · hello world"

    step "building"
    cargo build --release -p minibox-cli -p miniboxd 2>&1 | grep -E "^(Compiling|Finished|error)" | tail -1
    ok "binaries ready"; echo ""

    step "starting daemon"
    sudo ./target/release/miniboxd &>/dev/null &
    DAEMON_PID=$!
    trap 'sudo kill $DAEMON_PID 2>/dev/null; wait $DAEMON_PID 2>/dev/null || true' EXIT
    spin "waiting for daemon..." & SPIN=$!
    until sudo bash -c '[ -S /run/minibox/miniboxd.sock ]' 2>/dev/null; do sleep 0.05; done
    kill $SPIN 2>/dev/null; wait $SPIN 2>/dev/null || true
    printf "\r  ${G}✓${R}  daemon ready                        \n\n"

    step "pulling alpine"
    sudo ./target/release/minibox pull alpine; echo ""

    step "running container"
    sudo ./target/release/minibox run alpine -- /bin/echo "Hello from Minibox!"
    echo ""; ok "done"; echo ""

# Namespace isolation: build, start daemon, show UTS + PID + mount isolation vs host, stop daemon (Linux, root)
demo-isolation:
    #!/usr/bin/env bash
    set -euo pipefail
    C='\033[36m'; B='\033[1m'; G='\033[32m'; Y='\033[33m'; BL='\033[34m'; D='\033[2m'; R='\033[0m'
    hdr() { printf "\n${B}${C}  ╭─────────────────────────────────────────╮\n  │  %-39s  │\n  ╰─────────────────────────────────────────╯${R}\n\n" "$1"; }
    step() { printf "  ${B}${BL}▸${R}  ${B}%s${R}\n" "$1"; }
    ok()   { printf "  ${G}✓${R}  %s\n" "$1"; }
    row()  { printf "  ${D}%-10s${R}  ${Y}%-26s${R}  ${G}%-26s${R}\n" "$1" "$2" "$3"; }
    spin() {
        local frames=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏') i=0 msg="$1"
        while true; do printf "\r     ${C}${frames[$i]}${R}  %s" "$msg"; i=$(( (i+1)%10 )); sleep 0.08; done
    }

    if [[ "$(uname)" != "Linux" ]]; then
        printf "  ${C}demo-isolation requires Linux + root — try: just demo-pull${R}\n"; exit 1
    fi

    hdr "minibox · namespace isolation"

    step "building"
    cargo build --release -p minibox-cli -p miniboxd 2>&1 | grep -E "^(Compiling|Finished|error)" | tail -1
    ok "binaries ready"; echo ""

    step "starting daemon"
    sudo ./target/release/miniboxd &>/dev/null &
    DAEMON_PID=$!
    trap 'sudo kill $DAEMON_PID 2>/dev/null; wait $DAEMON_PID 2>/dev/null || true' EXIT
    spin "waiting for daemon..." & SPIN=$!
    until sudo bash -c '[ -S /run/minibox/miniboxd.sock ]' 2>/dev/null; do sleep 0.05; done
    kill $SPIN 2>/dev/null; wait $SPIN 2>/dev/null || true
    printf "\r  ${G}✓${R}  daemon ready                        \n\n"

    step "pulling alpine"
    sudo ./target/release/minibox pull alpine 2>&1 | tail -1
    ok "image ready"; echo ""

    step "sampling host and container environments"
    HOST_HN=$(hostname)
    HOST_OS=$(grep PRETTY_NAME /etc/os-release 2>/dev/null | cut -d'"' -f2 || uname -s)
    HOST_P1=$(cat /proc/1/comm)
    HOST_U="$(id -u) ($(id -un))"
    COUT=$(sudo ./target/release/minibox run alpine -- /bin/sh -c \
        'printf "%s\n%s\n%s\n%s\n" "$(hostname)" "$(cat /etc/alpine-release)" "$(cat /proc/1/comm)" "$(id -u)"' 2>/dev/null)
    C_HN=$(printf '%s' "$COUT" | sed -n '1p')
    C_VER=$(printf '%s' "$COUT" | sed -n '2p')
    C_P1=$(printf '%s' "$COUT" | sed -n '3p')
    C_U=$(printf '%s' "$COUT" | sed -n '4p')

    echo ""
    printf "  ${D}%-10s  ${B}${Y}%-26s  ${B}${G}%-26s${R}\n" "" "HOST" "CONTAINER (alpine)"
    printf "  ${D}%-10s  %-26s  %-26s${R}\n" "" "──────────────────────────" "──────────────────────────"
    row "hostname" "$HOST_HN" "$C_HN"
    row "os"       "$HOST_OS" "Alpine Linux $C_VER"
    row "pid 1"    "$HOST_P1" "$C_P1"
    row "user"     "$HOST_U"  "$C_U (root)"
    echo ""
    ok "done · three namespaces isolated, one shared kernel"
    echo ""

# ── Git ──────────────────────────────────────────────────────────────────────

# Stage all + commit (triggers pre-commit hook)
commit msg:
    git add -A
    git commit -m "{{msg}}"

# Push + clean non-critical artifacts on success
push *args:
    git push {{args}}
    cargo xtask clean-artifacts

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
