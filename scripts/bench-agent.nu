#!/usr/bin/env nu
# Benchmark analysis agent: report, compare, regress, cleanup, trigger.
# Usage: nu scripts/bench-agent.nu <subcommand> [options]

def main [] {
    print "bench-agent subcommands:"
    print "  report              summarise latest bench results"
    print "  compare [sha...]    compare bench runs (default: latest two VPS runs)"
    print "  regress             detect and explain regressions"
    print "  cleanup             clean up stale result files"
    print "  trigger             run benchmarks and analyse"
    print ""
    print "run 'nu scripts/bench-agent.nu <subcommand> --help' for options"
}

# Summarise latest bench results
def "main report" [
    --max-turns: int = 15   # Max agent turns
] {
    ^uv run ($env.FILE_PWD | path join "bench-agent.py") --max-turns ($max_turns | into string) report
}

# Compare bench runs
def "main compare" [
    ...sha: string           # Git SHAs to compare (default: latest two VPS runs)
    --max-turns: int = 15   # Max agent turns
] {
    let args = ["--max-turns" ($max_turns | into string) "compare"] ++ $sha
    ^uv run ($env.FILE_PWD | path join "bench-agent.py") ...$args
}

# Detect and explain regressions
def "main regress" [
    --threshold: float = 10.0  # Regression threshold % (default: 10)
    --max-turns: int = 15      # Max agent turns
] {
    let args = ["--max-turns" ($max_turns | into string) "regress" "--threshold" ($threshold | into string)]
    ^uv run ($env.FILE_PWD | path join "bench-agent.py") ...$args
}

# Clean up stale result files
def "main cleanup" [
    --dry-run               # Report only, don't delete
    --max-turns: int = 15   # Max agent turns
] {
    let args = ["--max-turns" ($max_turns | into string) "cleanup"]
    let args = if $dry_run { $args | append "--dry-run" } else { $args }
    ^uv run ($env.FILE_PWD | path join "bench-agent.py") ...$args
}

# Run benchmarks and analyse
def "main trigger" [
    --suite: string = ""    # Specific suite (codec, adapter, pull, run, exec, e2e)
    --vps                   # Run on VPS instead of locally
    --max-turns: int = 15   # Max agent turns
] {
    let args = ["--max-turns" ($max_turns | into string) "trigger"]
    let args = if ($suite | is-not-empty) { $args | append ["--suite" $suite] } else { $args }
    let args = if $vps { $args | append "--vps" } else { $args }
    ^uv run ($env.FILE_PWD | path join "bench-agent.py") ...$args
}
