#!/usr/bin/env nu
# preflight.nu — minibox environment validation (SessionStart hook)
#
# CANONICAL PREFLIGHT COMMAND: `cargo xtask doctor`
#   - Checks all required tools (cargo, just, rustup, cargo-nextest, gh, op)
#   - Reports CARGO_TARGET_DIR status
#   - On Linux: checks cgroups v2, overlay FS, and kernel version
#
# This script is a lightweight SessionStart hook that surfaces obvious
# missing dependencies at shell startup. For a full diagnostic run:
#   cargo xtask doctor          # tool + env checks
#   mbx doctor                  # adapter selection + tool + env checks

def check [label: string, pass: bool, detail: string = ""] {
    if $pass {
        print $"[ok]   ($label)"
    } else if $detail != "" {
        print $"[fail] ($label) — ($detail)"
    } else {
        print $"[fail] ($label)"
    }
    $pass
}

def note [label: string, detail: string = ""] {
    if $detail == "" {
        print $"[info] ($label)"
    } else {
        print $"[info] ($label) — ($detail)"
    }
}

print "=== minibox preflight ==="

let results = [
    (check "shell" ((which nu | length) > 0)),
    (check "cargo on PATH" ((which cargo | length) > 0)),
    (check "just on PATH" ((which just | length) > 0)),
    (check "rustup on PATH" ((which rustup | length) > 0)),
    (check "Rust toolchain active" ((do { cargo --version } | complete | get exit_code) == 0)),
    (check "CARGO_TARGET_DIR set" ($env | get -o CARGO_TARGET_DIR | is-not-empty)),
    (check "xtask available" (
        ((do { cargo metadata --no-deps --format-version 1 } | complete | get exit_code) == 0)
        and ("crates/xtask/Cargo.toml" | path exists)
    )),
    (check "op on PATH" ((which op | length) > 0)),
    (check "1Password authed" ((do { op account list } | complete | get exit_code) == 0)),
]

let git_status = (do { git status --porcelain } | complete | get stdout | str trim)
if ($git_status | is-empty) {
    note "git repo clean"
} else {
    note "git repo has local changes" "startup preflight ignores working tree dirtiness"
}

let failed = $results | where { |r| not $r } | length
let total = $results | length

print ""
if $failed == 0 {
    print $"preflight passed ($total)/($total)"
} else {
    print $"preflight ($total - $failed)/($total) - ($failed) checks failed"
}
