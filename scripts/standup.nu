#!/usr/bin/env nu
# Generate a time-blocked standup report from git activity across ~/dev/ repos.

def main [
    --hours: int = 24                   # Lookback window in hours
    --repos-dir: string = ""            # Root dir to scan for repos (default: ~/dev)
    --vault: string = ""                # Write report to Obsidian vault dir
    --no-sessions                       # Skip Claude session log analysis
] {
    let args = ["--hours" ($hours | into string)]

    let args = if ($repos_dir | is-not-empty) {
        $args | append ["--repos-dir" $repos_dir]
    } else {
        $args
    }

    let args = if ($vault | is-not-empty) {
        $args | append ["--vault" $vault]
    } else {
        $args
    }

    let args = if $no_sessions { $args | append "--no-sessions" } else { $args }

    ^uv run ($env.FILE_PWD | path join "scripts" "standup.py") ...$args
}
