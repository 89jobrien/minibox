#!/usr/bin/env nu
# Security and correctness pre-push review of changes vs a base branch.

def main [
    --base: string = "main"  # Base branch/ref to diff against
] {
    ^rust-script ($env.FILE_PWD | path join "ai-review.rs") --base $base
}
