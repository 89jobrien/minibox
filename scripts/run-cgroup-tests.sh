#!/bin/bash
# Run cgroup integration tests inside a properly delegated cgroup.
# Must be run as root.
set -euo pipefail

export PATH="/home/joe.linux/.cargo/bin:/root/.cargo/bin:$PATH"
export RUSTUP_HOME=/home/joe.linux/.rustup
export CARGO_HOME=/home/joe.linux/.cargo

SLICE=/sys/fs/cgroup/minibox-test-slice
LEAF=$SLICE/runner-leaf

echo "=== Cleaning up any previous test cgroups ==="
# Recursively move processes out and remove cgroup dirs, deepest first
cleanup_cgroup() {
    local dir="$1"
    [ -d "$dir" ] || return 0
    # Move any processes in this cgroup back to root
    if [ -f "$dir/cgroup.procs" ]; then
        while IFS= read -r pid; do
            [ -n "$pid" ] && echo "$pid" > /sys/fs/cgroup/cgroup.procs 2>/dev/null || true
        done < "$dir/cgroup.procs"
    fi
    # Recurse into children first
    for child in "$dir"/*/; do
        [ -d "$child" ] && cleanup_cgroup "$child"
    done
    rmdir "$dir" 2>/dev/null || true
}
cleanup_cgroup "$SLICE"

echo "=== Setting up test cgroup slice ==="
# Create the cgroup hierarchy WITHOUT putting this process in runner-leaf.
# The test binary will be the sole occupant of runner-leaf.
mkdir -p "$LEAF"

# Enable controllers at root and slice level (do this while no process is in runner-leaf)
for c in +memory +cpu +pids +io; do
    echo "$c" >> /sys/fs/cgroup/cgroup.subtree_control 2>/dev/null || true
done
for c in +memory +cpu +pids +io; do
    echo "$c" >> "$SLICE/cgroup.subtree_control" 2>/dev/null || true
done
# Do NOT enable controllers in runner-leaf here — that would make it a non-leaf cgroup
# and block processes from entering it (EBUSY). The test guard enables them after
# moving the process to a child.

echo "runner-leaf subtree_control: $(cat $LEAF/cgroup.subtree_control)"
echo "Script is running in cgroup: $(cat /proc/self/cgroup)"

echo "=== Building test binary ==="
cd /Users/joe/dev/minibox
cargo build -p miniboxd --test cgroup_tests 2>&1

# Find the test binary
TEST_BIN=$(cargo test -p miniboxd --test cgroup_tests --no-run --message-format=json 2>&1 \
    | python3 -c "
import sys,json
for line in sys.stdin:
    try:
        o = json.loads(line)
        if o.get('reason') == 'compiler-artifact' and o.get('profile', {}).get('test') and o.get('executable'):
            print(o['executable'])
    except: pass
" | tail -1)

echo "Test binary: $TEST_BIN"
echo "=== Running cgroup integration tests ==="

# Run the test binary in a subshell that is the SOLE process in runner-leaf.
# This avoids cgroup v2's "domain invalid" state when the test guard creates
# child cgroups: the test binary is the only process and can move itself to
# a child without leaving a non-leaf cgroup with processes behind.
(
    # Move this subshell into runner-leaf, then exec the test binary so that
    # no bash process remains in runner-leaf alongside the test binary.
    echo $BASHPID > "$LEAF/cgroup.procs"
    exec "$TEST_BIN" --test-threads=1 --nocapture
)
STATUS=$?

echo "=== Cleaning up ==="
if [ -d "$LEAF" ]; then
    for d in "$LEAF"/*/; do rmdir "$d" 2>/dev/null || true; done
    rmdir "$LEAF" 2>/dev/null || true
fi
rmdir "$SLICE" 2>/dev/null || true

exit $STATUS
