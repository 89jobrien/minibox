#!/usr/bin/env bash
# Run miniboxd under uftrace inside the Colima VM.
# Usage: colima ssh -- bash scripts/trace-lima.sh <BINARY_DIR> <ABS_TRACE>
set -euo pipefail

BINARY_DIR="$1"
ABS_TRACE="$2"

command -v uftrace >/dev/null 2>&1 || sudo apt-get install -y uftrace -q

uftrace record -P . --no-libcall -d "$ABS_TRACE" "$BINARY_DIR/miniboxd" &
INNER_PID=$!
sleep 2
"$BINARY_DIR/minibox" pull alpine 2>/dev/null || true
"$BINARY_DIR/minibox" run alpine -- /bin/echo 'uftrace smoke' 2>/dev/null || true
kill $INNER_PID 2>/dev/null || true
wait $INNER_PID 2>/dev/null || true
