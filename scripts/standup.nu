#!/usr/bin/env nu
# Generate a time-blocked standup report from git activity across ~/dev/ repos.

def main [
    --hours: int = 24                   # Lookback window in hours
    --repos-dir: string = ""            # Root dir to scan for repos (default: ~/dev)
    --vault: string = ""                # Write report to Obsidian vault dir
    --no-sessions                       # Skip Claude session log analysis
] {
    if (which devkit | is-empty) {
        error make {msg: "devkit not found on PATH; install devkit or update scripts/standup.nu"}
    }

    let repo_root = if ($repos_dir | is-empty) {
        $env.HOME | path join "dev"
    } else {
        $repos_dir | path expand
    }

    if not ($repo_root | path exists) {
        error make {msg: $"repos dir not found: ($repo_root)"}
    }

    let repos = (
        ls $repo_root
        | where type == dir
        | where { |row| ($row.name | path join ".git" | path exists) }
        | get name
    )

    let repo_args = ($repos | each { |repo| ["--repo" $repo] } | flatten)
    let args = ["standup" "--since" $"($hours)h" "--parallel"] | append $repo_args

    if $no_sessions {
        print "note: --no-sessions is accepted for compatibility; devkit standup controls session usage internally"
    }

    if ($vault | is-not-empty) {
        let report = (^devkit ...$args)
        print $report

        let vault_dir = ($vault | path expand)
        mkdir $vault_dir
        let report_path = ($vault_dir | path join $"standup-(date now | format date "%Y-%m-%d").md")
        $"($report)\n" | save --append $report_path
        print $"standup: appended report to ($report_path)"
    } else {
        ^devkit ...$args
    }
}
