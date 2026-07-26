#!/usr/bin/env nu
# ci-watch.nu — watch the latest GHA run with job-level detail
#
# Usage:
#   nu scripts/ci-watch.nu                  # watch run on current branch
#   nu scripts/ci-watch.nu --branch main    # watch run on a specific branch

def main [--branch: string] {
    if ($branch | is-empty) {
        cargo xtask ci-watch
    } else {
        cargo xtask ci-watch --branch $branch
    }
}
