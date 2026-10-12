use std/assert

let repo_root = ($env.FILE_PWD | path dirname)
let sandbox = ($env.TMPDIR | path join $"minibox-ingest-docs-test-(random uuid)")
let docs_dir = ($sandbox | path join docs)
let nested_docs_dir = ($docs_dir | path join nested)
let bin_dir = ($sandbox | path join bin)
let capture = ($sandbox | path join ingested.txt)

mkdir $nested_docs_dir $bin_dir
"plain markdown" | save ($docs_dir | path join guide.md)
"minibox markdown" | save ($nested_docs_dir | path join architecture.mbx.md)
"not markdown" | save ($docs_dir | path join ignored.txt)

let fake_kgx = ($bin_dir | path join kgx)
'#!/usr/bin/env nu

def main [command: string] {
    if $command != "ingest" {
        exit 2
    }

    open --raw /dev/stdin | save --append $env.KGX_CAPTURE
}
' | save $fake_kgx
^chmod +x $fake_kgx

let result = (with-env {
    PATH: ($env.PATH | prepend $bin_dir)
    KGX_CAPTURE: $capture
} {
    cd $sandbox
    do {
        ^nu --no-config-file ($repo_root | path join ingest_docs.nu)
    } | complete
})

assert equal $result.exit_code 0 $result.stderr
let ingested = (open --raw $capture)
assert ($ingested | str contains "plain markdown")
assert ($ingested | str contains "minibox markdown")
assert not ($ingested | str contains "not markdown")

rm --recursive $sandbox
