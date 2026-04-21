#!/usr/bin/env nu
# Agent run history and benchmark dashboard — native Nu tables.
# Reads ~/.minibox/agent-runs.jsonl and bench/results/.

def fmt-ts [iso: string] {
    $iso | str substring 0..15 | str replace "T" " "
}

def fmt-dur [secs: float] {
    if $secs < 60 {
        $"($secs | math round --precision 1)s"
    } else {
        let m = ($secs / 60 | math floor)
        let s = ($secs mod 60 | math round)
        $"($m)m($s)s"
    }
}

def preview [text] {
    let t = ($text | default "" | into string)
    let first = ($t | lines | first | default "")
    if ($first | str length) > 40 {
        $"($first | str substring 0..40)…"
    } else {
        $first
    }
}

def load-runs [] {
    let log_file = ($env.HOME | path join ".minibox" "agent-runs.jsonl")
    if not ($log_file | path exists) { return [] }

    # Parse JSONL, deduplicate by run_id (latest entry wins)
    open $log_file
    | lines
    | where { |l| ($l | str trim | is-not-empty) }
    | each { |l| $l | from json }
    | group-by run_id
    | items { |_id, entries|
        # complete > running
        let complete = ($entries | where status == "complete")
        if ($complete | length) > 0 { $complete | last } else { $entries | last }
    }
}

def agents-summary [] {
    let runs = (load-runs)
    if ($runs | length) == 0 {
        print "no agent runs found in ~/.minibox/agent-runs.jsonl"
        return
    }

    print "=== Agent Summary ==="
    let summary = (
        $runs
        | group-by script
        | items { |script, entries|
            let completed = ($entries | where status == "complete")
            let avg_dur = if ($completed | length) > 0 {
                $completed | get duration_s | math avg | math round --precision 1
            } else { 0.0 }
            let last_run = ($entries | sort-by run_id | last)
            {
                script: $script
                runs: ($entries | length)
                avg_s: $avg_dur
                last_run: (fmt-ts $last_run.run_id)
                last_output: (preview ($last_run | get -o output | default ""))
            }
        }
        | sort-by script
    )
    print ($summary | table)

    print ""
    print "=== Recent Runs (last 20) ==="
    let recent = (
        $runs
        | sort-by run_id
        | last 20
        | each { |r|
            let status_str = match $r.status {
                "complete" => "done"
                "crash"    => "CRASH"
                _          => "live"
            }
            let dur = if ($r | get -o duration_s | is-empty) { "" } else {
                fmt-dur $r.duration_s
            }
            {
                time:    (fmt-ts $r.run_id)
                script:  $r.script
                status:  $status_str
                dur:     $dur
                output:  (preview ($r | get -o output | default ""))
            }
        }
    )
    print ($recent | table)
}

def bench-summary [] {
    let latest_path = "bench/results/latest.json"
    if not ($latest_path | path exists) {
        print "no bench results found (bench/results/latest.json)"
        return
    }

    let latest = (open $latest_path)
    let sha     = ($latest | get -o metadata.git_sha | default "?")
    let host    = ($latest | get -o metadata.hostname  | default "?")
    let ts      = ($latest | get -o metadata.timestamp | default "?" | into string | str substring 0..19)

    print $"=== Benchmarks — ($sha | str substring 0..8) @ ($host) ($ts) ==="

    let rows = (
        $latest.suites
        | each { |suite|
            $suite.tests | each { |t|
                let avg  = ($t | get -o stats.avg | default null)
                let p95  = ($t | get -o stats.p95 | default null)
                let unit = ($t | get -o unit | default "µs")
                let unit_str = if $unit == "nanos" { "ns" } else { "µs" }
                {
                    suite: $suite.name
                    test:  $t.name
                    avg:   (if $avg != null { $"($avg | math round)($unit_str)" } else { "-" })
                    p95:   (if $p95 != null { $"($p95 | math round)($unit_str)" } else { "-" })
                    iters: ($t | get -o iterations | default 0)
                }
            }
        }
        | flatten
    )

    print ($rows | table)

    # Storage stats
    let jsonl = "bench/results/bench.jsonl"
    if ($jsonl | path exists) {
        let run_count = (open $jsonl | lines | where { |l| $l | str trim | is-not-empty } | length)
        let size_kb   = (ls $jsonl | get size.0 | into int) / 1024
        print $"\n($run_count) runs in bench.jsonl  •  ($size_kb | math round)KB"
    }
}

def main [
    --agents    # Show only agent history
    --bench     # Show only benchmark results
] {
    if $agents {
        agents-summary
    } else if $bench {
        bench-summary
    } else {
        agents-summary
        print ""
        bench-summary
    }
}
