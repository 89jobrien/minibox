#!/usr/bin/env nu
# Collect test metrics from cargo test runs and display as Nu tables.
# Optionally saves a timestamped JSONL run record to artifacts/reports/.

def parse-test-output [raw: string] {
    # Parse cargo test JSON lines (--format json)
    let events = (
        $raw
        | lines
        | where { |l| $l | str trim | is-not-empty }
        | each { |l|
            try { $l | from json } catch { null }
        }
        | where { |e| $e != null }
    )

    let tests = ($events | where { |e| ($e | get -o type | default "") == "test" })
    let ok      = ($tests | where { |e| ($e | get -o event | default "") == "ok"      } | length)
    let failed  = ($tests | where { |e| ($e | get -o event | default "") == "failed"  } | length)
    let ignored = ($tests | where { |e| ($e | get -o event | default "") == "ignored" } | length)
    let total   = $ok + $failed + $ignored

    let failures = (
        $tests
        | where { |e| ($e | get -o event | default "") == "failed" }
        | get -o name
        | default []
    )

    {
        total:    $total
        ok:       $ok
        failed:   $failed
        ignored:  $ignored
        failures: $failures
    }
}

def run-cargo-tests [crates: list<string>] {
    # Run each crate's tests and collect results
    $crates | each { |p|
        print $"running tests: ($p)"
        let result = (^cargo test -p $p --lib -- --format json -Z unstable-options | complete)
        let parsed = (parse-test-output $result.stdout)
        {
            crate:   $p
            total:   $parsed.total
            ok:      $parsed.ok
            failed:  $parsed.failed
            ignored: $parsed.ignored
        }
    }
}

def load-bench-summary [] {
    let latest = "bench/results/latest.json"
    if not ($latest | path exists) { return [] }

    let data = (open $latest)
    $data.suites | each { |suite|
        let valid = ($suite.tests | where { |t| ($t | get -o stats | is-not-empty) })
        {
            suite:  $suite.name
            tests:  ($suite.tests | length)
            with_stats: ($valid | length)
            avg_us: (
                if ($valid | length) > 0 {
                    $valid | each { |t| $t | get -o stats.avg | default 0.0 } | math avg | math round --precision 1
                } else { null }
            )
        }
    }
}

def save-run [results: list, reports_dir: string] {
    let ts = (date now | format date "%Y%m%dT%H%M%SZ")
    let run_dir = ($reports_dir | path join $ts)
    mkdir $run_dir

    let sha = (^git rev-parse HEAD | complete).stdout | str trim
    let branch = (^git rev-parse --abbrev-ref HEAD | complete).stdout | str trim

    let meta = {
        timestamp: $ts
        git_sha:   $sha
        branch:    $branch
        results:   $results
    }

    $meta | to json | save ($run_dir | path join "meta.json")
    print $"saved run to ($run_dir)/meta.json"
}

def main [
    --reports-dir: string = "artifacts/reports"  # Directory to save run records
    --save                                        # Save a timestamped run record
    --crates: string = ""                         # Comma-separated crate list (default: standard set)
    --self-test                                   # Run self-test and exit
] {
    if $self_test {
        print "collect-metrics self-test: OK"
        return
    }

    let default_crates = ["minibox" "minibox-macros" "minibox-cli" "daemonbox"]
    let target_crates = if ($crates | is-not-empty) {
        $crates | split row ","
    } else {
        $default_crates
    }

    # --- Test results ---
    print "=== Test Results ===\n"
    let results = (run-cargo-tests $target_crates)
    $results | table
    print ""

    # Totals row
    let total_tests   = ($results | get total   | math sum)
    let total_ok      = ($results | get ok      | math sum)
    let total_failed  = ($results | get failed  | math sum)
    let total_ignored = ($results | get ignored | math sum)
    print $"TOTAL  ($total_tests) tests  ✓ ($total_ok)  ✗ ($total_failed)  — ($total_ignored) ignored"

    # Show any failures
    let all_failures = (
        $results
        | each { |r|
            let fs = ($r | get -o failures | default [])
            $fs | each { |f| $"($r.crate)::($f)" }
        }
        | flatten
    )
    if ($all_failures | length) > 0 {
        print "\nFailed tests:"
        $all_failures | each { |f| print $"  ✗ ($f)" }
    }

    # --- Bench summary ---
    print "\n=== Bench Summary (latest.json) ===\n"
    let bench = (load-bench-summary)
    if ($bench | length) > 0 {
        $bench | table
    } else {
        print "no bench results (run: cargo xtask bench)"
    }

    # --- Save run record ---
    if $save {
        if not ($reports_dir | path exists) {
            mkdir $reports_dir
        }
        save-run $results $reports_dir
    }

    if $total_failed > 0 {
        error make { msg: $"($total_failed) test(s) failed" }
    }
}
