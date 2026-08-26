#!/usr/bin/env nu
# Audit generated docs: freshness (source changed after the doc's SHA),
# home-collisions, and dead links.
#
# Exits non-zero when anything actionable is found, so it can gate a commit.
#
#   nu doc-audit.nu                 # freshness + collisions (fast, offline)
#   nu doc-audit.nu --links         # also HTTP-check every external link
#   nu doc-audit.nu --json          # machine-readable, for another tool to consume

def doc-homes [] {
    [
        {name: "memory-bank", glob: ".ctx/memory-bank/*.md"}
        {name: "top-level",   glob: "*.md"}
        {name: "docs-core",   glob: "docs/core/*.md"}
    ]
}

# Split YAML frontmatter off a doc. Returns null when the doc has none —
# an unstamped doc is a finding, not an error.
def read-frontmatter [path: string] {
    let raw = (open --raw $path)
    let lines = ($raw | lines)
    if ($lines | is-empty) or ($lines | first) != "---" {
        return null
    }
    let rest = ($lines | skip 1)
    let ends = ($rest | enumerate | where item == "---" | get index)
    if ($ends | is-empty) { return null }
    let end = ($ends | first)
    let block = ($rest | first $end | str join (char nl))
    try { $block | from yaml } catch { null }
}

# A doc is stale when any of its sources picked up a commit that is a
# descendant of the SHA the doc was generated from. Comparing SHAs alone
# would also fire when the doc is *ahead* of the source, which is fine.
def source-drift [doc_sha: string, sources: list] {
    mut drifted = []
    # An unknown SHA (typo, or a commit that never landed / was rebased away)
    # would make every ancestry test fail and the doc would look permanently
    # fresh. Surface it instead of silently passing.
    let known = (do { git cat-file -e $"($doc_sha)^{commit}" } | complete | get exit_code)
    if $known != 0 {
        return [{source: "(frontmatter)", reason: $"source_sha ($doc_sha | str substring 0..8) is not a commit in this repo"}]
    }
    for src in $sources {
        if not ($src | path exists) {
            $drifted = ($drifted | append {source: $src, reason: "source path missing"})
            continue
        }
        let latest = (do { git log -1 --format=%H -- $src } | complete | get stdout | str trim)
        if ($latest | is-empty) or $latest == $doc_sha { continue }
        let is_desc = (do { git merge-base --is-ancestor $doc_sha $latest } | complete | get exit_code)
        if $is_desc == 0 {
            let subject = (do { git log -1 --format=%s -- $src } | complete | get stdout | str trim)
            $drifted = ($drifted | append {source: $src, reason: $"changed in ($latest | str substring 0..8) — ($subject)"})
        }
    }
    $drifted
}

def collect-docs [] {
    mut docs = []
    for home in (doc-homes) {
        for f in (glob $home.glob) {
            let rel = ($f | path relative-to (pwd))
            # Top-level home is ALL-CAPS only; README/CLAUDE-style files that are
            # hand-written land here too, so record and let the caller decide.
            $docs = ($docs | append {
                path: $rel
                home: $home.name
                base: ($rel | path basename | str replace ".mbx.md" "" | str replace ".md" "")
                front: (read-frontmatter $rel)
            })
        }
    }
    $docs
}

# One topic, one home. A base name living in both top-level and docs/core
# means two docs drift apart and readers can't tell which is canonical.
def find-collisions [docs: list] {
    $docs
    | where home in ["top-level" "docs-core"]
    | group-by base
    | items {|base, group|
        if ($group | length) > 1 {
            {base: $base, paths: ($group | get path)}
        }
    }
    | compact
}

def extract-links [path: string] {
    open --raw $path
    | parse --regex '\((?<url>https?://[^)\s]+)\)'
    | get url
    | append (open --raw $path | parse --regex '<(?<url>https?://[^>\s]+)>' | get url)
    | uniq
}

def check-links [docs: list] {
    let all = ($docs | each {|d| extract-links $d.path | each {|u| {doc: $d.path, url: $u}}} | flatten)
    # Dedupe by URL so a link repeated across docs costs one request.
    let unique = ($all | get url | uniq)
    mut results = []
    for u in $unique {
        let code = (do { curl -sS -L -o /dev/null -w "%{http_code}" --max-time 15 $u } | complete | get stdout | str trim)
        if $code !~ '^[23]' {
            let docs_with = ($all | where url == $u | get doc | uniq)
            $results = ($results | append {url: $u, status: $code, docs: $docs_with})
        }
    }
    $results
}

def main [--links, --json] {
    let docs = (collect-docs)
    let stamped = ($docs | where front != null)
    let unstamped = ($docs | where front == null)

    mut stale = []
    for d in $stamped {
        let sha = ($d.front | get -o source_sha)
        let sources = ($d.front | get -o sources | default [])
        if $sha == null or ($sources | is-empty) { continue }
        let drift = (source-drift $sha $sources)
        if not ($drift | is-empty) {
            $stale = ($stale | append {doc: $d.path, source_sha: ($sha | str substring 0..8), drift: $drift})
        }
    }

    let collisions = (find-collisions $docs)
    let dead = if $links { check-links $docs } else { [] }

    if $json {
        print ({stale: $stale, collisions: $collisions, dead_links: $dead, unstamped: ($unstamped | get path)} | to json)
        return
    }

    print $"docs audited: ($docs | length) \(($stamped | length) stamped, ($unstamped | length) unstamped\)"

    if not ($collisions | is-empty) {
        print $"\n(ansi red)COLLISIONS(ansi reset) — same topic in two homes; pick one canonical location:"
        for c in $collisions { print $"  ($c.base): ($c.paths | str join ' vs ') " }
    }

    if not ($dead | is-empty) {
        print $"\n(ansi red)DEAD LINKS(ansi reset):"
        for l in $dead { print $"  [($l.status)] ($l.url)\n      in: ($l.docs | str join ', ')" }
    }

    if ($stale | is-empty) {
        print $"\n(ansi green)No stale docs.(ansi reset) Every stamped doc's sources are unchanged since its source_sha."
    } else {
        print $"\n(ansi yellow)STALE — source changed after the doc was generated(ansi reset):"
        for s in $stale {
            print $"  ($s.doc)  \(generated from ($s.source_sha)\)"
            for d in $s.drift { print $"      ($d.source): ($d.reason)" }
        }
    }

    if not ($unstamped | is-empty) {
        print $"\n(ansi yellow)UNSTAMPED(ansi reset) — no frontmatter, so freshness cannot be checked:"
        for u in $unstamped { print $"  ($u.path)" }
    }

    let failures = (($stale | length) + ($collisions | length) + ($dead | length))
    if $failures > 0 { exit 1 }
}
