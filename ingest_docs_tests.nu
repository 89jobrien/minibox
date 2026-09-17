#!/usr/bin/env nu

def check [condition: bool, message: string] {
    if not $condition {
        error make {msg: $message}
    }
}

def run-ingest [repo: path, docs: path, kgx: path, dry_run: bool] {
    let args = if $dry_run { ["--dry-run"] } else { [] }
    with-env {KGX_BIN: ($kgx | into string)} {
        do { ^nu ($repo | path join "ingest_docs.nu") --docs-root $docs ...$args } | complete
    }
}

def main [] {
    let repo = ($env.FILE_PWD)
    let temp = (^mktemp -d | str trim)
    let docs = ($temp | path join "docs")
    let bin = ($temp | path join "bin")
    let marker = ($temp | path join "kgx-called")
    mkdir ($docs | path join "nested") $bin
    "second" | save ($docs | path join "z.md")
    "first" | save ($docs | path join "nested/a.md")

    let kgx = ($bin | path join "kgx")
    $'#!/bin/sh
printf "%s\\n" "$*" >> "($marker)"
cat >/dev/null
' | save $kgx
    ^chmod +x $kgx

    let dry = (run-ingest $repo $docs $kgx true)
    check ($dry.exit_code == 0) $"dry-run failed: ($dry.stderr)"
    check (not ($marker | path exists)) "dry-run invoked kgx"
    let lines = ($dry.stdout | lines | where {|line| $line starts-with "Ingesting " })
    check ($lines == ["Ingesting nested/a.md" "Ingesting z.md"]) $"unexpected ordering: ($lines)"

    $'#!/bin/sh
cat >/dev/null
exit 17
' | save -f $kgx
    ^chmod +x $kgx
    let kgx_failure = (run-ingest $repo $docs $kgx false)
    check ($kgx_failure.exit_code != 0) "kgx failure was not propagated"

    let unreadable = ($docs | path join "unreadable.md")
    "cannot read" | save $unreadable
    ^chmod 000 $unreadable
    $'#!/bin/sh
cat >/dev/null
' | save -f $kgx
    ^chmod +x $kgx
    let input_failure = (run-ingest $repo $docs $kgx false)
    ^chmod 600 $unreadable
    check ($input_failure.exit_code != 0) "input read failure was not propagated"

    rm -rf $temp
    print "ingest_docs tests passed"
}
