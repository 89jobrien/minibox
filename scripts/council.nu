#!/usr/bin/env nu
# Multi-role AI code review of the current branch.
# Runs 3 roles (core) or 5 roles (extensive) and synthesises findings.

def main [
    --base: string = "main"     # Base branch/ref to diff against
    --mode: string = "core"     # Review mode: core (3 roles) or extensive (5 roles)
    --no-synthesis              # Skip the synthesis step
] {
    if $mode not-in ["core" "extensive"] {
        error make { msg: $"--mode must be 'core' or 'extensive', got '($mode)'" }
    }

    let args = ["--base" $base "--mode" $mode]
    let args = if $no_synthesis { $args | append "--no-synthesis" } else { $args }

    ^uv run ($env.FILE_PWD | path join "council.py") ...$args
}
