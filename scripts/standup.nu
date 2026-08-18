#!/usr/bin/env nu
# Generate a time-blocked standup report from git activity across ~/dev/ repos.
# Gathers repo activity + recent Claude session excerpts natively in Nu, then
# hands the assembled context to `claude -p` for narrative synthesis.

def log-dir [] {
    $env.HOME | path join ".minibox"
}

def log-start [run_id: string, script: string, args: record] {
    let dir = (log-dir)
    mkdir $dir
    {run_id: $run_id, script: $script, args: $args, status: "running"}
    | to json -r
    | save --append ($dir | path join "agent-runs.jsonl")
}

def log-complete [run_id: string, script: string, args: record, status: string, duration_s: float, output: string] {
    let dir = (log-dir)
    {
        run_id: $run_id
        script: $script
        args: $args
        status: $status
        duration_s: ($duration_s | math round --precision 2)
        output: $output
    }
    | to json -r
    | save --append ($dir | path join "agent-runs.jsonl")
}

def find-repo-root [] {
    mut p = (pwd | path expand)
    loop {
        if ($p | path join ".git" | path exists) {
            return $p
        }
        let parent = ($p | path dirname)
        if $parent == $p {
            return null
        }
        $p = $parent
    }
}

def git-in [cwd: string, args: list<string>] {
    let result = (do { ^git -C $cwd ...$args } | complete)
    if $result.exit_code != 0 { "" } else { $result.stdout | str trim }
}

def find-active-repos [repos_dir: string, hours: int] {
    let since = $"($hours) hours ago"
    ls $repos_dir
    | where type == dir
    | where { |d| $d.name | path join ".git" | path exists }
    | each { |d|
        let commit_log = (git-in $d.name ["log" $"--since=($since)" "--all" "--format=%ai|%h|%s"])
        if ($commit_log | is-empty) {
            null
        } else {
            let files_raw = (git-in $d.name ["log" $"--since=($since)" "--all" "--name-only" "--format="])
            let files = (
                $files_raw | lines | where { |f| $f | str trim | is-not-empty } | uniq | sort
            )
            {
                path: $d.name
                name: ($d.name | path basename)
                commit_log: $commit_log
                files: $files
                branch: (git-in $d.name ["rev-parse" "--abbrev-ref" "HEAD"])
                status: (git-in $d.name ["status" "--short"])
                stash: (git-in $d.name ["stash" "list"])
            }
        }
    }
    | compact
}

def build-repo-section [repo: record] {
    let commits = ($repo.commit_log | lines)
    mut lines = [
        $"## ($repo.name)"
        $"($commits | length) commit\(s\) · ($repo.files | length) file\(s\) touched"
        ""
        "Commits (timestamp | hash | subject):"
    ]
    for c in $commits {
        $lines = ($lines | append $"  ($c)")
    }
    if ($repo.files | length) > 0 {
        let shown = ($repo.files | first 20 | str join ", ")
        $lines = ($lines | append $"\nFiles: ($shown)")
    }
    if ($repo.status | is-not-empty) {
        $lines = ($lines | append $"\nUncommitted:\n($repo.status)")
    }
    if ($repo.stash | is-not-empty) {
        $lines = ($lines | append $"\nStashes:\n($repo.stash)")
    }
    $lines | str join "\n"
}

def find-claude-sessions [hours: int] {
    let sessions_dir = ($env.HOME | path join ".claude" "projects")
    if not ($sessions_dir | path exists) { return "" }

    let cutoff = ((date now) - ($hours * 1hr))
    let recent = (
        glob ($sessions_dir | path join "**" "*.jsonl")
        | each { |p| {path: $p, modified: (ls $p | get modified.0)} }
        | where modified > $cutoff
        | sort-by modified --reverse
        | first 3
    )
    if ($recent | length) == 0 { return "" }

    $recent
    | each { |r|
        let msgs = (try { open $r.path | lines | last 60 } catch { [] })
        let excerpts = (
            $msgs
            | each { |line|
                let entry = (try { $line | from json } catch { null })
                if $entry == null { null } else {
                    let e = $entry
                    if ($e | get -o type) != "assistant" { null } else {
                        let content = ($e | get -o message.content)
                        let kind = ($content | describe)
                        let is_list = ($kind | str starts-with "list") or ($kind | str starts-with "table")
                        let text = if $is_list {
                            $content
                            | where { |b| ($b | get -o type) == "text" }
                            | get -o text.0
                            | default ""
                        } else if ($content | describe) == "string" {
                            $content
                        } else {
                            ""
                        }
                        let trimmed = ($text | str trim)
                        if ($trimmed | str length) > 20 {
                            $trimmed | str substring 0..250
                        } else {
                            null
                        }
                    }
                }
            }
            | compact
        )
        if ($excerpts | length) == 0 {
            null
        } else {
            let session_tag = ($r.path | path dirname | path basename | str substring (-8)..)
            let shown = ($excerpts | last 4 | each { |m| $"  - ($m)" } | str join "\n")
            $"session \(($session_tag)\):\n($shown)"
        }
    }
    | compact
    | str join "\n\n"
}

def build-prompt [repo_context: string, session_context: string, hours: int] {
    let session_section = if ($session_context | is-not-empty) {
        $"\n\nRecent Claude session excerpts:\n($session_context)"
    } else {
        ""
    }
    $"Generate a standup report for the last ($hours)h structured as time blocks.

FORMAT — produce exactly these four sections:

## State at start
One short paragraph. Where were things at the beginning of this window — what was the last stable state before this batch of work began? Infer from the oldest commit in the window and the files touched.

## Timeline
Group commits into 1-hour calendar blocks \(e.g. 09:00–10:00, 10:00–11:00\).
Each block on one line: `HH:00–HH:00  description of work  \(files: x, y, z\)`
Oldest block first \(chronological order\). Use the actual commit timestamps provided.
If commits span multiple days, prefix each day's blocks with the date on its own line.
Summarise what was accomplished in each block — not just the commit subject verbatim.

## Current state + next
Two to three sentences. What is the current state of the codebase/work, and what is the logical next step? Include any open stashes or in-flight threads.

## Concerns
Risks, drift, technical debt, or open questions worth flagging. If none, say 'None.'

Rules: be terse, cite short hashes where useful, infer intent from commit messages.

--- REPOSITORY ACTIVITY ---
($repo_context)($session_section)"
}

def append-to-timeline [output: string, now: string] {
    let root = (find-repo-root)
    if $root == null { return null }
    let timeline = ($root | path join "docs" "STANDUP.md")
    if not ($timeline | path exists) { return null }
    let entry = $"\n## ($now)\n\n($output | str trim)\n\n---\n"
    $entry | save --append $timeline
    {timeline: $timeline, root: $root}
}

def main [
    --hours: int = 24                   # Lookback window in hours
    --repos-dir: string = ""            # Root dir to scan for repos (default: ~/dev)
    --vault: string = ""                # Write report to Obsidian vault dir
    --no-sessions                       # Skip Claude session log analysis
] {
    let repos_dir = if ($repos_dir | is-empty) {
        $env.HOME | path join "dev"
    } else {
        $repos_dir
    }
    let default_vault = ($env.HOME | path join "Documents" "Obsidian Vault" "Reports")

    let now = (date now)
    let now_disp = ($now | format date "%Y-%m-%d %H:%M")
    print $"\nStandup — last ($hours)h — ($now_disp)\n"

    let active_repos = (find-active-repos $repos_dir $hours)
    if ($active_repos | length) == 0 {
        print $"No activity in ($repos_dir) in the last ($hours)h."
        return
    }

    let names = ($active_repos | get name | str join ", ")
    print $"Active repos \(($active_repos | length)\): ($names)\n"

    let all_repo_names = (
        ls $repos_dir
        | where type == dir
        | where { |d| $d.name | path join ".git" | path exists }
        | get name
    )
    let active_names = ($active_repos | get path)
    let inactive = (
        $all_repo_names
        | where { |n| $n not-in $active_names }
        | each { |n| $n | path basename }
    )

    mut repo_sections = ($active_repos | each { |r| build-repo-section $r })
    if ($inactive | length) > 0 {
        $repo_sections = ($repo_sections | append $"_No activity in: ($inactive | str join ', ')_")
    }
    let repo_context = ($repo_sections | str join "\n\n---\n\n")

    let session_context = if $no_sessions { "" } else { (find-claude-sessions $hours) }

    let run_id = ($now | format date "%+")
    let args = {hours: $hours, repos_dir: $repos_dir}
    log-start $run_id "standup" $args

    let prompt = (build-prompt $repo_context $session_context $hours)
    let start = (date now)
    let result = (do { ^claude -p $prompt } | complete)
    let elapsed = (((date now) - $start) / 1sec)

    if $result.exit_code != 0 {
        print -e $"error: standup failed: ($result.stderr)"
        log-complete $run_id "standup" $args "crash" $elapsed $result.stderr
        exit 1
    }

    let standup_output = $result.stdout
    print $standup_output

    let frontmatter = $"---\ntype: standup\ndate: ($now | format date '%Y-%m-%d')\nhour: \"($now | format date '%H:00')\"\nrepos_active: ($active_repos | length)\nwindow_hours: ($hours)\n---\n\n"
    let header = $"# Standup — ($now_disp)\n\n_window: ($hours)h_\n\n"
    let full_report = $"($frontmatter)($header)($standup_output)\n\n---\n\n($repo_context)"

    let timeline_result = (append-to-timeline $standup_output $now_disp)
    if $timeline_result != null {
        let rel = ($timeline_result.timeline | str replace $"($timeline_result.root)/" "")
        print $"\nAppended to: ($rel)"
    }

    let vault_dir = if ($vault | is-not-empty) {
        $vault
    } else if ($default_vault | path exists) {
        $default_vault
    } else {
        ""
    }

    if ($vault_dir | is-not-empty) {
        mkdir $vault_dir
        let filename = $"($now | format date '%Y-%m-%d %H:00').md"
        let out_path = ($vault_dir | path join $filename)
        $full_report | save --force $out_path
        print $"\nWritten to: ($out_path)"
    }

    log-complete $run_id "standup" $args "complete" $elapsed $full_report
}
