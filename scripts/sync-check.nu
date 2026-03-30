#!/usr/bin/env nu
# Pre-push sync check: fetch origin, detect ahead/behind/diverged, auto-resolve conflicts.
# Exits 0 if safe to push, 1 if manual intervention needed.

def main [
    --dry-run               # Report only, make no changes
    --base: string = "origin/main"  # Remote ref to check against
] {
    let args = ["--base" $base]
    let args = if $dry_run { $args | append "--dry-run" } else { $args }

    ^uv run ($env.FILE_PWD | path join "scripts" "sync-check.py") ...$args
}
