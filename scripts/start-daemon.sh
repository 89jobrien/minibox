#!/usr/bin/env bash
# Thin wrapper: resolves the miniboxd binary and delegates to it.
#
# Restart/stop logic is now built into miniboxd --restart.
# Use MINIBOX_ADAPTER to select the adapter suite.
#
# Usage:
#   ./scripts/start-daemon.sh
#   ./scripts/start-daemon.sh --adapter colima
#   MINIBOX_ADAPTER=native ./scripts/start-daemon.sh

set -euo pipefail

adapter=""

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
Start miniboxd, replacing any existing instance.

Usage:
  ./scripts/start-daemon.sh [--adapter <name>]

The --adapter flag sets MINIBOX_ADAPTER. If omitted, miniboxd auto-selects
the adapter (smolvm, falling back to krun if smolvm is not installed).

Adapter selection and restart are handled by miniboxd internally.
Run `mbx doctor` to see which adapters are compiled into this build.
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
  echo "run: cargo build --release -p miniboxd" >&2
  exit 1
fi

if [[ -n "$adapter" ]]; then
  export MINIBOX_ADAPTER="$adapter"
fi

echo "Starting miniboxd (MINIBOX_ADAPTER=${MINIBOX_ADAPTER:-auto})..."
exec sudo --preserve-env=MINIBOX_ADAPTER,LIMA_HOME "$binary" --restart
