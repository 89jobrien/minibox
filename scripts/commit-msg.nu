#!/usr/bin/env nu
# Generate a conventional commit message from staged changes using Claude.

def main [
    --stage (-a)    # Stage all changes (git add -A) before generating
    --commit (-c)   # Commit with the generated message after confirming
    --yes (-y)      # Skip confirmation and commit immediately
] {
    let args = []
    let args = if $stage  { $args | append "-a" }  else { $args }
    let args = if $commit { $args | append "-c" }  else { $args }
    let args = if $yes    { $args | append "-y" }  else { $args }

    ^uv run ($env.FILE_PWD | path join "scripts" "commit-msg.py") ...$args
}
