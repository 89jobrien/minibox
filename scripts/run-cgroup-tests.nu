#!/usr/bin/env nu
# Run cgroup v2 integration tests under a properly delegated cgroup hierarchy.
# Requires Linux + root (or systemd user delegation).

def main [] {
    let slice   = "/sys/fs/cgroup/minibox-test-slice"
    let leaf    = $"($slice)/runner-leaf"
    let cgroup2 = "/sys/fs/cgroup"

    # Check cgroups v2
    let mounts = (^mount | complete).stdout
    if not ($mounts | str contains "cgroup2") {
        error make { msg: "cgroups v2 not mounted at /sys/fs/cgroup" }
    }

    # Clean up prior test cgroups
    print "cleaning up prior test cgroups..."
    let stale = (^bash -c $"find ($cgroup2) -name 'minibox-test-*' -type d 2>/dev/null" | lines)
    for dir in $stale {
        if ($dir | str trim | is-not-empty) {
            ^rmdir ($dir | str trim) | ignore
        }
    }

    # Create slice + leaf
    print $"creating cgroup hierarchy: ($leaf)"
    ^mkdir -p $leaf

    # Enable controllers in slice
    let controllers = "+cpu +memory +pids"
    $controllers | ^tee $"($slice)/cgroup.subtree_control" | ignore

    # Build cgroup_tests binary
    print "building cgroup_tests..."
    let rustup_home = ($env.HOME | path join ".rustup")
    let cargo_home  = ($env.HOME | path join ".cargo")

    let build_result = (^env RUSTUP_HOME=$rustup_home CARGO_HOME=$cargo_home
        cargo build --release -p miniboxd --test cgroup_tests | complete)
    if $build_result.exit_code != 0 {
        error make { msg: $"cargo build failed:\n($build_result.stderr)" }
    }

    # Find test binary
    let test_bin = (
        ^bash -c "ls -t target/release/deps/cgroup_tests-* 2>/dev/null | grep -v '\\.d$' | head -1"
        | str trim
    )
    if ($test_bin | is-empty) {
        error make { msg: "could not find cgroup_tests binary in target/release/deps/" }
    }
    print $"found test binary: ($test_bin)"

    # Move runner into leaf cgroup
    print $"joining cgroup ($leaf)..."
    ^bash -c $"echo $$ > ($leaf)/cgroup.procs"

    # Run tests (single-threaded to avoid cgroup domain-invalid errors)
    print "running cgroup tests (single-threaded)..."
    let test_result = (^env RUSTUP_HOME=$rustup_home CARGO_HOME=$cargo_home
        $test_bin --test-threads=1 --nocapture | complete)

    # Parse and display summary
    let lines = $test_result.stdout | lines
    let summary = $lines | where { |l| ($l | str contains "test result") }
    for line in $summary {
        print $line
    }

    # Cleanup
    print "cleaning up test cgroups..."
    let test_dirs = (^bash -c $"find ($cgroup2) -name 'minibox-test-*' -type d 2>/dev/null" | lines)
    for dir in $test_dirs {
        if ($dir | str trim | is-not-empty) {
            ^rmdir ($dir | str trim) | ignore
        }
    }

    if $test_result.exit_code != 0 {
        error make { msg: "cgroup tests failed" }
    }

    print "cgroup tests passed ✓"
}
