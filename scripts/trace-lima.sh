#!/usr/bin/env bash
# Run miniboxd under uftrace inside the Colima VM.
# Usage: colima ssh -- bash scripts/trace-lima.sh <BINARY_DIR> <ABS_TRACE>
set -euo pipefail

BINARY_DIR="$1"
ABS_TRACE="$2"

command -v uftrace >/dev/null 2>&1 || sudo apt-get install -y uftrace -q

# miniboxd requires root; run uftrace under sudo and write trace to a temp dir
# owned by root, then chown it back so the calling user can read the report.
SUDO_TRACE="$ABS_TRACE"
sudo uftrace record --force -P . --no-libcall -d "$SUDO_TRACE" "$BINARY_DIR/miniboxd" &
INNER_PID=$!
sleep 2
sudo "$BINARY_DIR/minibox" pull alpine 2>/dev/null || true
sudo "$BINARY_DIR/minibox" run alpine -- /bin/echo 'uftrace smoke' 2>/dev/null || true
sudo kill $INNER_PID 2>/dev/null || true
wait $INNER_PID 2>/dev/null || true
# Restore ownership so the calling (non-root) user can read uftrace report output.
sudo chown -R "$(id -u):$(id -g)" "$SUDO_TRACE" 2>/dev/null || true
