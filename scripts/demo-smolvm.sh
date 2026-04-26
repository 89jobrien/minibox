#!/usr/bin/env bash
# demo-smolvm.sh — Progressive smolvm/krun demo for minibox.
#
# Runs 7 increasingly complex tasks inside ephemeral smolvm VMs to
# demonstrate the krun adapter's capabilities.
#
# Tasks 1-6 are self-contained. Task 7 (codex) requires a pre-encrypted
# dotenvx .env with CODEX_AUTH_JSON (see below).
#
# Codex setup (one-time):
#   1. Run `codex login` on the host to create ~/.codex/auth.json
#   2. mkdir -p /tmp/smolvm-codex-env && cd /tmp/smolvm-codex-env
#   3. python3 -c "import json; print('CODEX_AUTH_JSON=' + json.dumps(json.load(open('$HOME/.codex/auth.json'))))" > .env
#   4. dotenvx encrypt
#   5. Export: CODEX_DOTENVX_DIR=/tmp/smolvm-codex-env
#
# Usage:
#   ./scripts/demo-smolvm.sh          # run all tasks
#   ./scripts/demo-smolvm.sh 3        # run only task 3
#   ./scripts/demo-smolvm.sh 4 7      # run tasks 4 through 7
set -euo pipefail

# ── Colours ───────────────────────────────────────────────────────────────
BOLD='\033[1m'
GREEN='\033[32m'
RED='\033[31m'
CYAN='\033[36m'
DIM='\033[2m'
RESET='\033[0m'

IMAGE="ubuntu:24.04"
SHARE_DIR="$(mktemp -d)"
PASSED=0
FAILED=0
SKIPPED=0

cleanup() { rm -rf "$SHARE_DIR"; }
trap cleanup EXIT

banner() { printf "\n${BOLD}${CYAN}── Task %s: %s${RESET}\n" "$1" "$2"; }

run_task() {
    local num="$1" name="$2"
    shift 2
    banner "$num" "$name"
    local start
    start=$(date +%s)
    if "$@"; then
        local elapsed=$(( $(date +%s) - start ))
        printf "${GREEN}  PASS${RESET} ${DIM}(%ds)${RESET}\n" "$elapsed"
        (( PASSED++ ))
    else
        local elapsed=$(( $(date +%s) - start ))
        printf "${RED}  FAIL${RESET} ${DIM}(%ds)${RESET}\n" "$elapsed"
        (( FAILED++ ))
    fi
}

should_run() {
    local num="$1"
    [[ -z "${TASK_MIN:-}" ]] && return 0
    (( num >= TASK_MIN && num <= TASK_MAX ))
}

# ── Parse args ────────────────────────────────────────────────────────────
TASK_MIN="${1:-}"
TASK_MAX="${2:-$TASK_MIN}"

# ── Preflight ─────────────────────────────────────────────────────────────
if ! command -v smolvm &>/dev/null; then
    printf "${RED}smolvm not found on PATH. Install: brew install smolvm${RESET}\n"
    exit 1
fi
printf "${DIM}smolvm %s | image: %s | share: %s${RESET}\n" \
    "$(smolvm --version 2>/dev/null | awk '{print $2}')" "$IMAGE" "$SHARE_DIR"

# ══════════════════════════════════════════════════════════════════════════
# Task 1: Basic command execution
# ══════════════════════════════════════════════════════════════════════════
task1() {
    smolvm machine run --image "$IMAGE" --timeout 30s -- \
        echo "Hello from smolvm/krun!"
}

# ══════════════════════════════════════════════════════════════════════════
# Task 2: System introspection (kernel, cpus, memory)
# ══════════════════════════════════════════════════════════════════════════
task2() {
    smolvm machine run --image "$IMAGE" --timeout 30s -- \
        sh -c 'uname -a && head -4 /etc/os-release && echo "cpus: $(nproc)" && free -h | head -2'
}

# ══════════════════════════════════════════════════════════════════════════
# Task 3: Outbound networking (apt install + HTTP request)
# ══════════════════════════════════════════════════════════════════════════
task3() {
    smolvm machine run --net --image "$IMAGE" --timeout 60s -- \
        sh -c 'apt-get update -qq >/dev/null 2>&1 && \
               apt-get install -y -qq curl >/dev/null 2>&1 && \
               curl -s https://httpbin.org/ip'
}

# ══════════════════════════════════════════════════════════════════════════
# Task 4: Volume mount — bidirectional file I/O via virtiofs
# ══════════════════════════════════════════════════════════════════════════
task4() {
    echo "hello from host ($(date))" > "$SHARE_DIR/host-greeting.txt"
    smolvm machine run --image "$IMAGE" -v "$SHARE_DIR:/data" --timeout 30s -- \
        sh -c 'echo "--- host file ---" && \
               head /data/host-greeting.txt && \
               echo "reply from VM ($(uname -r))" > /data/vm-reply.txt'

    # Verify the VM wrote back to the host
    printf "  host reads back: "
    head "$SHARE_DIR/vm-reply.txt"
}

# ══════════════════════════════════════════════════════════════════════════
# Task 5: Compile + run a C program, export binary to host
# ══════════════════════════════════════════════════════════════════════════
task5() {
    smolvm machine run --net --image "$IMAGE" --cpus 4 --mem 4096 \
        -v "$SHARE_DIR:/data" --timeout 120s -- \
        sh -c '
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq gcc >/dev/null 2>&1
tee /tmp/fib.c <<CEOF
#include <stdio.h>
int main() {
    printf("Hello from C compiled inside smolvm/krun!\\n");
    int a=0, b=1, c;
    for (int i=0; i<10; i++) {
        printf("fib(%d) = %d\\n", i, a);
        c=a+b; a=b; b=c;
    }
    return 0;
}
CEOF
gcc -O2 -o /data/fib-from-vm /tmp/fib.c
/data/fib-from-vm
'
    # Verify binary on host
    printf "  host: "
    file "$SHARE_DIR/fib-from-vm"
}

# ══════════════════════════════════════════════════════════════════════════
# Task 6: Install Rust toolchain, compile + run a Rust binary
# ══════════════════════════════════════════════════════════════════════════
task6() {
    smolvm machine run --net --image "$IMAGE" --cpus 4 --mem 4096 \
        -v "$SHARE_DIR:/data" --timeout 300s -- \
        sh -c '
set -e
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq curl build-essential >/dev/null 2>&1
curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal >/dev/null 2>&1
export PATH="$HOME/.cargo/bin:$PATH"
echo "rustc: $(rustc --version)"
tee /tmp/demo.rs <<REOF
fn fib(n: u64) -> u64 {
    match n { 0 => 0, 1 => 1, _ => fib(n-1) + fib(n-2) }
}
fn main() {
    println!("Hello from Rust compiled inside smolvm/krun!");
    for i in 0..15 { println!("fib({i}) = {}", fib(i)); }
    println!("arch: {}", std::env::consts::ARCH);
    println!("os:   {}", std::env::consts::OS);
}
REOF
rustc -O -o /data/rust-demo /tmp/demo.rs
/data/rust-demo
'
    # Verify binary on host
    printf "  host: "
    file "$SHARE_DIR/rust-demo"
}

# ══════════════════════════════════════════════════════════════════════════
# Task 7: Run OpenAI Codex CLI inside the VM (dotenvx-encrypted secrets)
# ══════════════════════════════════════════════════════════════════════════
CODEX_DOTENVX_DIR="${CODEX_DOTENVX_DIR:-/tmp/smolvm-codex-env}"
# npm is used inside the VM (bun not available in base ubuntu image)
_NPM="npm"

task7() {
    if [ ! -f "$CODEX_DOTENVX_DIR/.env" ]; then
        printf "  SKIP: %s/.env not found (see setup instructions)\n" "$CODEX_DOTENVX_DIR"
        return 1
    fi

    local priv_key
    priv_key=$(dotenvx keypair DOTENV_PRIVATE_KEY -f "$CODEX_DOTENVX_DIR/.env" 2>/dev/null)
    if [ -z "$priv_key" ]; then
        printf "  SKIP: could not extract DOTENV_PRIVATE_KEY\n"
        return 1
    fi

    smolvm machine run --net --image "$IMAGE" --cpus 4 --mem 4096 \
        -v "$CODEX_DOTENVX_DIR:/secrets" \
        -e "DOTENV_PRIVATE_KEY=$priv_key" \
        --timeout 180s -- sh -c "
set -e
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq curl git >/dev/null 2>&1

# Install dotenvx
curl -sfS https://dotenvx.sh | sh >/dev/null 2>&1

# Install node + codex
curl -fsSL https://deb.nodesource.com/setup_22.x | bash - >/dev/null 2>&1
apt-get install -y -qq nodejs >/dev/null 2>&1
$_NPM install -g @openai/codex@0.121.0 >/dev/null 2>&1
echo \"codex \$(codex --version) + dotenvx \$(dotenvx --version)\"

# Decrypt OAuth tokens into codex config
mkdir -p /root/.codex
dotenvx run -f /secrets/.env -- sh -c 'echo \"\$CODEX_AUTH_JSON\" > /root/.codex/auth.json'

# Run codex on a simple coding task
mkdir -p /tmp/workspace && cd /tmp/workspace && git init -q
codex exec --full-auto 'write hello.py that prints fibonacci up to 10 terms, then run it'
"
}

# ── Run ───────────────────────────────────────────────────────────────────
should_run 1 && run_task 1 "echo (basic execution)"           task1 || true
should_run 2 && run_task 2 "system introspection"              task2 || true
should_run 3 && run_task 3 "outbound networking"               task3 || true
should_run 4 && run_task 4 "volume mount (bidirectional I/O)"  task4 || true
should_run 5 && run_task 5 "compile C program"                 task5 || true
should_run 6 && run_task 6 "compile Rust program"              task6 || true
should_run 7 && run_task 7 "codex CLI (dotenvx secrets)"       task7 || true

# ── Summary ───────────────────────────────────────────────────────────────
printf "\n${BOLD}── Summary${RESET}\n"
printf "  ${GREEN}passed: %d${RESET}  ${RED}failed: %d${RESET}\n" "$PASSED" "$FAILED"
[[ "$FAILED" -eq 0 ]] && exit 0 || exit 1
