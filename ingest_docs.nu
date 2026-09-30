#!/usr/bin/env nu

def discover-docs [root: path] {
    glob ($root | path join "**/*.md") | sort
}

def main [--dry-run, --docs-root: path] {
    let root = ($docs_root | default ((pwd) | path join "docs"))
    let docs = (discover-docs $root)
    let kgx = ($env.KGX_BIN? | default "kgx")

    for doc in $docs {
        let relative = ($doc | path relative-to $root)
        print $"Ingesting ($relative)"
        if not $dry_run {
            let contents = (open --raw $doc)
            let result = (do { $contents | ^$kgx ingest } | complete)
            if $result.exit_code != 0 {
                let detail = ($result.stderr | str trim)
                error make {msg: $"kgx ingest failed for ($relative) with exit code ($result.exit_code): ($detail)"}
            }
        }
    }
}
