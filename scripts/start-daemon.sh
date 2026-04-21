#!/usr/bin/env bash
# Start miniboxd with the selected adapter, resolving the binary from the
# shared target cache when CARGO_TARGET_DIR is set via direnv.
#
# Usage:
#   ./scripts/start-daemon.sh
#   ./scripts/start-daemon.sh --adapter colima
#   ./scripts/start-daemon.sh --adapter native

set -euo pipefail

adapter="colima"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --adapter)
      if [[ $# -lt 2 ]]; then
        echo "error: --adapter requires a value" >&2
        exit 1
      fi
      adapter="$2"
      shift 2
      ;;
    -h|--help)
      cat <<'EOF'
Start miniboxd with the selected adapter, killing any existing instance first.

Usage:
  ./scripts/start-daemon.sh [--adapter <name>]

Examples:
  ./scripts/start-daemon.sh
  ./scripts/start-daemon.sh --adapter colima
  ./scripts/start-daemon.sh --adapter native
EOF
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

target_dir="${CARGO_TARGET_DIR:-$HOME/.minibox/cache/target}"
binary="$target_dir/release/miniboxd"

if [[ ! -x "$binary" ]]; then
  echo "error: miniboxd not found at $binary" >&2
  echo "run: cargo build --release" >&2
  exit 1
fi

if pgrep -x miniboxd >/dev/null 2>&1; then
  echo "Stopping existing miniboxd instance(s)..."
  pkill -x miniboxd || true
  sleep 1
fi

export MINIBOX_ADAPTER="$adapter"
export LIMA_HOME="${LIMA_HOME:-$HOME/.colima/_lima}"

echo "Starting miniboxd (MINIBOX_ADAPTER=$MINIBOX_ADAPTER)..."
exec sudo --preserve-env=MINIBOX_ADAPTER,LIMA_HOME "$binary"
