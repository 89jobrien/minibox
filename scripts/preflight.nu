#!/usr/bin/env nu
# preflight.nu — minibox environment validation

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

print "=== minibox preflight ==="

let results = [
    (check "shell" ((which nu | length) > 0)),
    (check "cargo on PATH" ((which cargo | length) > 0)),
    (check "just on PATH" ((which just | length) > 0)),
    (check "rustup on PATH" ((which rustup | length) > 0)),
    (check "Rust toolchain active" ((do { cargo --version } | complete | get exit_code) == 0)),
    (check "CARGO_TARGET_DIR set" ($env | get -i CARGO_TARGET_DIR | is-not-empty)),
    (check "xtask available" ((do { cargo xtask --help } | complete | get exit_code) == 0)),
    (check "op on PATH" ((which op | length) > 0)),
    (check "1Password authed" ((do { op account list } | complete | get exit_code) == 0)),
    (check "git repo clean" (do { git status --porcelain } | complete | get stdout | str trim | is-empty)),
]

let failed = $results | where { |r| not $r } | length
let total = $results | length

print ""
if $failed == 0 {
    print $"preflight passed ($total)/($total)"
} else {
    print $"preflight ($total - $failed)/($total) - ($failed) checks failed"
}
