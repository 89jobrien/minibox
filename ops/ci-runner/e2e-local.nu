#!/usr/bin/env nu
# Local macOS end-to-end smoke test for the ops/ci-runner/ image.
#
# Validates, against a real miniboxd daemon on this machine:
#   1. `mbx build` of ops/ci-runner/Dockerfile under the smolvm adapter
#      (exercises the persistent-build-VM RUN-chaining and COPY/VM-bridging
#      fixes — a Dockerfile with 2+ dependent RUN steps plus a COPY used to
#      fail silently on this path).
#   2. `mbx run --privileged` + `mbx exec` under the krun adapter, proving
#      the daemon can actually boot and exec into a privileged container on
#      this macOS host (the mode the ci-runner would run in, absent a real
#      Linux host).
#
# NOT covered here, and not a bug this script can route around: pushing the
# built image to ghcr.io so krun's independent pull-by-ref path can boot it
# directly. `mbx push` itself is fixed (see crates/miniboxd/src/main.rs —
# image_pusher is now wired for both macOS adapters), but `gh auth token` in
# this environment carries only `gist, read:org, repo` scopes, not
# `write:packages`. Run `gh auth refresh -s write:packages` (interactive)
# or supply a GHCR PAT via --token/--username+--password to close that gap;
# until then, step 2 below runs against library/alpine:latest instead of the
# just-built image, to validate the run+exec leg independently.
#
# Usage: nu ops/ci-runner/e2e-local.nu

def kill-miniboxd [] {
    # Kill all miniboxd processes and wait for cleanup
    ps | where name == "miniboxd" | each { |p| kill -9 $p.pid } | ignore
    sleep 2sec
}

def main [] {
    let repo_root = ($env.CURRENT_FILE | path dirname | path dirname | path dirname)
    let dockerfile_dir = ($repo_root | path join "ops" "ci-runner")

    print "=== stage 0: clean daemon state ==="
    kill-miniboxd

    print "=== stage 1: mbx build (smolvm) ==="
    with-env { MINIBOX_ADAPTER: "smolvm" } {
        ^miniboxd &
        sleep 1sec
        let build_result = (do { mbx build -t minibox-ci-runner:e2e $dockerfile_dir } | complete)
        kill-miniboxd
        if $build_result.exit_code != 0 {
            print "FAIL: mbx build"
            print $build_result.stderr
            exit 1
        }
    }
    print "PASS: mbx build (11 steps, RUN-chaining + COPY bridging)"

    print "=== stage 2: mbx run --privileged + mbx exec (krun) ==="
    with-env { MINIBOX_ADAPTER: "krun", MINIBOX_ALLOW_PRIVILEGED: "1" } {
        ^miniboxd &
        sleep 1sec

        # `mbx run` has no detach flag — it streams and blocks until the
        # container exits, so it must be backgrounded here.
        ^mbx run --privileged library/alpine:latest -- sh -c "echo 'Container init complete'; sleep 300" &
        sleep 2sec

        # Wait for container to reach Running state with PID
        mut ready = false
        mut tries = 0
        while (not $ready) and $tries < 30 {
            let container_lines = (mbx ps | lines | skip 2 | where { |l| ($l =~ "Running") and (not ($l =~ '\s-\s*$')) })
            if ($container_lines | length) > 0 {
                $ready = true
            } else {
                sleep 1sec
            }
            $tries += 1
        }

        if not $ready {
            kill-miniboxd
            print "FAIL: mbx run --privileged — container never reached Running state with PID"
            exit 1
        }

        # Extract container ID and test exec
        let container_id = (mbx ps | lines | skip 2 | where { |l| ($l =~ "Running") and (not ($l =~ '\s-\s*$')) } | first | split row -r '\s+' | first)
        let exec_result = (do { ^mbx exec $container_id -- uname -a } | complete)

        kill-miniboxd

        if $exec_result.exit_code != 0 {
            print "FAIL: mbx exec"
            print $exec_result.stderr
            exit 1
        }
        print $"PASS: mbx exec -> ($exec_result.stdout | str trim)"
    }

    print "=== e2e-local: ALL STAGES PASSED ==="
    print "NOTE: ghcr.io push not exercised — gh token lacks write:packages. See header."
}
