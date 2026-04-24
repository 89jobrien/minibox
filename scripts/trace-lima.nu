#!/usr/bin/env nu
# Profile miniboxd under uftrace inside the Colima VM.
# Usage: nu scripts/trace-lima.nu <binary-dir> <abs-trace-dir>
# Run via: colima ssh -- nu scripts/trace-lima.nu /path/to/bins /tmp/trace-out

def main [
    binary_dir: string   # Path to compiled binaries (e.g. target/release)
    abs_trace: string    # Absolute path for trace output directory
] {
    # Install uftrace if missing
    if (^which uftrace | complete).exit_code != 0 {
        print "installing uftrace..."
        ^sudo apt-get install -y uftrace
    }

    let miniboxd = ($binary_dir | path join "miniboxd")
    let minibox  = ($binary_dir | path join "minibox")

    if not ($miniboxd | path exists) {
        error make { msg: $"miniboxd not found at ($miniboxd)" }
    }

    # Clean trace dir
    if ($abs_trace | path exists) {
        ^rm -rf $abs_trace
    }
    ^mkdir -p $abs_trace

    print "starting miniboxd under uftrace..."
    let record_cmd = $"sudo uftrace record -d ($abs_trace) ($miniboxd)"
    let daemon_pid = (^bash -c $"($record_cmd) &\necho $!" | str trim)
    print $"miniboxd pid: ($daemon_pid)"

    # Brief settle
    ^sleep 2

    print "running smoke test (pull + run)..."
    let pull_result = (^sudo $minibox pull alpine | complete)
    if $pull_result.exit_code != 0 {
        print $"warning: pull failed — ($pull_result.stderr)"
    }

    let run_result = (^sudo $minibox run alpine -- /bin/true | complete)
    if $run_result.exit_code != 0 {
        print $"warning: run failed — ($run_result.stderr)"
    }

    # Stop profiler
    print "stopping uftrace..."
    ^sudo kill -INT $daemon_pid
    ^sleep 1

    # Fix ownership so the calling user can read results
    let caller = (^whoami | str trim)
    ^sudo chown -R $"($caller):($caller)" $abs_trace

    print $"trace saved to: ($abs_trace)"
    print "view with: uftrace report -d <trace-dir>"
}
