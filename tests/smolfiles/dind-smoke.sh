#!/bin/bash
# DinD smoke test inside a smolvm guest.
#
# Mirrors crux/dev/smoke.crux: start an outer miniboxd, run a privileged
# container with the daemon bind-mounted, delegate a cgroup subtree from
# *inside* that container, start an inner daemon there, and have the inner
# daemon pull and run an image.
#
# The guest provides what the native adapter needs: root, cgroups v2 rw, and
# overlayfs.
set -eu

# Musl binaries, overridable so the same script runs in a guest that mounts the
# repo somewhere other than /workspace (e.g. colima, which shares $HOME).
BIN_DIR="${MINIBOX_BIN_DIR:-/workspace/target/aarch64-unknown-linux-musl/debug}"
RUN_DIR="${MINIBOX_RUN_DIR:-/tmp/minibox-smoke-run}"
DATA_DIR="${MINIBOX_DATA_DIR:-/tmp/minibox-smoke-data}"
INNER_DATA=/tmp/minibox-dind-data
INNER_RUN=/tmp/minibox-dind-run

echo "--- host prerequisites ---"
id -u
grep cgroup2 /proc/mounts
grep -c overlay /proc/filesystems
test -x "$BIN_DIR/miniboxd"
test -x "$BIN_DIR/mbx"

# Clean any overlay a previous aborted run left mounted. `rm -rf` on a live
# merge point fails with EBUSY, which would mask the real test result.
for merged in "$DATA_DIR"/containers/*/merged "$DATA_DIR"/containers/*/upper; do
    mountpoint -q "$merged" 2>/dev/null && umount -lf "$merged" 2>/dev/null || true
done

rm -rf "$RUN_DIR" "$DATA_DIR" "$INNER_DATA" "$INNER_RUN"
mkdir -p "$RUN_DIR" "$DATA_DIR" "$INNER_DATA" "$INNER_RUN"

CGROUP_NAME="minibox-smoke-$$"

# The outer daemon. Bind mounts and privileged mode are denied by default.
#
# MINIBOX_ADAPTER=native is essential: the guest is a smolvm VM, so the `smolvm`
# binary is present and would otherwise be auto-selected. This test is about
# the native adapter running inside a Linux guest, not about nesting smolvm.
#
# MINIBOX_CGROUP_ROOT is deliberately NOT set: the daemon then resolves its own
# supervisor cgroup. Forcing a root makes it create a plain directory whose
# subtree_control was never enabled, and the first pids.max write EPERMs.
MINIBOX_ADAPTER=native \
    MINIBOX_RUN_DIR="$RUN_DIR" \
    MINIBOX_DATA_DIR="$DATA_DIR" \
    MINIBOX_ALLOW_BIND_MOUNTS=true \
    MINIBOX_ALLOW_PRIVILEGED=true \
    "$BIN_DIR/miniboxd" >/tmp/outer.log 2>&1 &
OUTER_PID=$!
trap 'kill $OUTER_PID 2>/dev/null || true' EXIT
sleep 3
grep -E "adapter suite selected|policy|supervisor" /tmp/outer.log || true

mbx() { MINIBOX_RUN_DIR="$RUN_DIR" MINIBOX_DATA_DIR="$DATA_DIR" "$BIN_DIR/mbx" "$@"; }

echo "--- outer pull ---"
mbx pull alpine:latest

echo "--- run privileged DinD container ---"
SCRIPT=$(
    cat <<'INNER'
set -eu
CGROUP_NAME=$(cat /tmp/dind-cgroup-name)
mkdir -p "/sys/fs/cgroup/$CGROUP_NAME"
echo '+memory +cpu +pids' > "/sys/fs/cgroup/$CGROUP_NAME/cgroup.subtree_control"
echo "  delegated: $(cat /sys/fs/cgroup/$CGROUP_NAME/cgroup.subtree_control)"

MINIBOX_DATA_DIR=/minibox-data \
  MINIBOX_RUN_DIR=/minibox-run \
  MINIBOX_CGROUP_ROOT="/sys/fs/cgroup/$CGROUP_NAME" \
  RUST_LOG=error \
  /usr/local/bin/miniboxd > /tmp/inner.log 2>&1 &
DAEMON_PID=$!

i=0
while [ "$i" -lt 100 ] && [ ! -S /minibox-run/miniboxd.sock ]; do
  sleep 0.1; i=$((i+1))
done
[ -S /minibox-run/miniboxd.sock ] || { echo 'inner daemon socket timeout' >&2; cat /tmp/inner.log >&2; kill "$DAEMON_PID" || true; exit 1; }
echo "  inner daemon listening"

MINIBOX_SOCKET_PATH=/minibox-run/miniboxd.sock /usr/local/bin/minibox pull alpine >/dev/null
OUT=$(MINIBOX_SOCKET_PATH=/minibox-run/miniboxd.sock /usr/local/bin/minibox run alpine -- /bin/sh -c 'echo dind-ok && uname -r')
echo "$OUT" | grep -q dind-ok || { echo "unexpected output: $OUT" >&2; kill "$DAEMON_PID" || true; exit 2; }
echo "  nested container output: $OUT"
kill "$DAEMON_PID" 2>/dev/null || true
wait "$DAEMON_PID" 2>/dev/null || true
echo dind-ok
INNER
)
echo "$CGROUP_NAME" >/tmp/dind-cgroup-name

mbx run --privileged \
    -v "$BIN_DIR/miniboxd:/usr/local/bin/miniboxd:ro" \
    -v "$BIN_DIR/mbx:/usr/local/bin/minibox:ro" \
    -v /sys/fs/cgroup:/sys/fs/cgroup \
    -v "$INNER_DATA:/minibox-data" \
    -v "$INNER_RUN:/minibox-run" \
    alpine:latest -- /bin/sh -c "$SCRIPT"

echo "--- outer ps ---"
mbx ps
echo "SMOKE RESULT: PASS"
