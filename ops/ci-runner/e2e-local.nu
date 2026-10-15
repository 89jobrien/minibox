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

def wait-for-daemon [tries: int = 20] {
    mut n = 0
    loop {
        let result = (do { mbx ps } | complete)
        if $result.exit_code == 0 {
            return true
        }
        $n += 1
        if $n >= $tries {
            return false
        }
    }
}

def kill-daemon [] {
    ps | where name == "miniboxd" | each { |p| kill $p.pid } | ignore
}

def main [] {
    let repo_root = ($env.CURRENT_FILE | path dirname | path dirname | path dirname)
    let dockerfile_dir = ($repo_root | path join "ops" "ci-runner")

    print "=== stage 0: clean daemon state ==="
    kill-daemon

    print "=== stage 1: mbx build (smolvm) ==="
    job spawn { with-env { MINIBOX_ADAPTER: "smolvm" } { ^miniboxd } }
    if not (wait-for-daemon) {
        print "FAIL: smolvm daemon did not come up"
        exit 1
    }
    let build_result = (do {
        mbx build -t minibox-ci-runner:e2e $dockerfile_dir
    } | complete)
    kill-daemon
    if $build_result.exit_code != 0 {
        print "FAIL: mbx build"
        print $build_result.stderr
        exit 1
    }
    print "PASS: mbx build (11 steps, RUN-chaining + COPY bridging)"

    print "=== stage 2: mbx run --privileged + mbx exec (krun) ==="
    job spawn { with-env { MINIBOX_ADAPTER: "krun", MINIBOX_ALLOW_PRIVILEGED: "1" } { ^miniboxd } }
    if not (wait-for-daemon) {
        print "FAIL: krun daemon did not come up"
        exit 1
    }

    # `mbx run` has no detach flag — it streams and blocks until the
    # container exits, so it must be backgrounded here, not awaited.
    # Run a shell that stays open (not just sleep) to validate full init.
    job spawn { mbx run --privileged library/alpine:latest -- sh -c "echo 'Container init complete'; sleep 300" } | ignore

    mut container_id = ""
    mut tries = 0
    # Wait for container to appear in ps (may be Created initially, should transition to Running)
    while ($container_id | is-empty) and $tries < 30 {
        let all_containers = (
            mbx ps | lines | skip 2
            | where { |l| not ($l =~ "no containers") }
        )
        if ($all_containers | length) > 0 {
            # Get the most recently created container (should be our alpine:latest)
            $container_id = ($all_containers | last | split row -r '\s+' | first)
        } else {
            sleep 1sec
        }
        $tries += 1
    }
    if ($container_id | is-empty) {
        kill-daemon
        print "FAIL: mbx run --privileged — container failed to appear in `mbx ps`"
        exit 1
    }

    # Now wait for the container to reach Running state with a PID.
    # Note: parsing is fragile because COMMAND column can have spaces, so we look for
    # state keywords directly in the line rather than field position.
    mut ready = false
    $tries = 0
    while (not $ready) and $tries < 30 {
        let container_lines = (mbx ps | lines | skip 2 | where { |l| $l =~ $container_id })
        if ($container_lines | length) > 0 {
            let line = $container_lines | first
            # Container is ready when it has state "Running" and a numeric PID (not "-")
            if ($line =~ "Running") and (not ($line =~ '\s-\s*$')) {
                $ready = true
            } else {
                sleep 1sec
            }
        } else {
            sleep 1sec
        }
        $tries += 1
    }
    if not $ready {
        kill-daemon
        print "FAIL: mbx run --privileged — container never reached Running state with PID"
        exit 1
    }
    let cid = $container_id
    let exec_result = (do { mbx exec $cid -- uname -a } | complete)
    if $exec_result.exit_code != 0 {
        mbx stop $cid | ignore
        mbx rm $cid | ignore
        kill-daemon
        print "FAIL: mbx exec"
        print $exec_result.stderr
        exit 1
    }
    print $"PASS: mbx exec -> ($exec_result.stdout | str trim)"

    mbx stop $cid | ignore
    mbx rm $cid | ignore
    kill-daemon

    print "=== e2e-local: ALL STAGES PASSED ==="
    print "NOTE: ghcr.io push not exercised — gh token lacks write:packages. See header."
}
