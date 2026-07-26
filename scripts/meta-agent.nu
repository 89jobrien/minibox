#!/usr/bin/env nu
# Design and spawn parallel agents from a task description.
# Fetches + caches Claude Agent SDK docs (24h TTL), designs 2–5 agents, runs concurrently.

def main [
    task?: string              # Task description (or pipe via stdin)
    --no-synthesis             # Skip the synthesis step
    --refresh-docs             # Force re-fetch SDK docs even if cache is fresh
] {
    # Allow task from stdin if not provided as arg
    let task_str = if ($task | is-empty) {
        if not ($in | is-empty) { $in } else { "" }
    } else {
        $task
    }

    if ($task_str | is-empty) {
        error make { msg: "provide a task description as argument or via stdin" }
    }

    let args = [$task_str]
    let args = if $no_synthesis  { $args | append "--no-synthesis"  } else { $args }
    let args = if $refresh_docs  { $args | append "--refresh-docs"  } else { $args }

    ^uv run ($env.FILE_PWD | path join "meta-agent.py") ...$args
}
