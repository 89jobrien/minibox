#!/usr/bin/env nu
# AI-powered container failure diagnosis.
# Gathers journalctl logs, mount state, cgroup hierarchy, and runtime files.

def main [
    --container: string = ""  # Container ID to focus on (optional)
    --lines: int = 200        # Daemon log lines to fetch
] {
    let args = ["--lines" ($lines | into string)]
    let args = if ($container | is-not-empty) {
        $args | append ["--container" $container]
    } else {
        $args
    }

    ^uv run ($env.FILE_PWD | path join "scripts" "diagnose.py") ...$args
}
