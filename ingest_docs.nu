#!/usr/bin/env nu

def discover-docs [root: path] {
    glob ($root | path join "**/*.md") | sort
}

def main [--dry-run] {
    let root = ((pwd) | path join "docs")
    let docs = (discover-docs $root)

    for doc in $docs {
        let relative = ($doc | path relative-to $root)
        print $"Ingesting ($relative)"
        if not $dry_run {
            open --raw $doc | ^kgx ingest
        }
    }
}
