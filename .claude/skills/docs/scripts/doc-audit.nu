#!/usr/bin/env nu
# Audit generated docs: freshness (delegated to `cargo xtask docs audit
# --stale`, the git-derived staleness engine), home-collisions, and dead
# links.
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

# Freshness is derived from git history by `cargo xtask docs audit --stale`
# (xtask/src/docs_audit.rs) — walks each doc's substantive commit history
# (skipping restamp-only commits) and compares against each declared
# source's last change. Returns the StalenessReport, parsed from JSON.
def stale-report [] {
    let result = (do { cargo xtask docs audit --stale } | complete)
    if $result.exit_code != 0 {
        print -e $"(ansi red)docs audit --stale failed:(ansi reset)\n($result.stderr)"
        exit 1
    }
    $result.stdout | from json
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

    # Plans, designs, specs, and other nested docs may also carry provenance.
    # Include them for freshness/path validation without treating them as a
    # canonical doc home for collision detection.
    for f in (glob "docs/**/*.md") {
        let rel = ($f | path relative-to (pwd))
        if not ($docs | any {|d| $d.path == $rel}) {
            $docs = ($docs | append {
                path: $rel
                home: "docs-other"
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

    let report = (stale-report)
    let stale = $report.stale

    let collisions = (find-collisions $docs)
    let dead = if $links { check-links $docs } else { [] }

    if $json {
        print ({stale: $stale, collisions: $collisions, dead_links: $dead, unstamped: ($unstamped | get path)} | to json)
        return
    }

    print $"docs audited: ($docs | length) \(($stamped | length) stamped, ($unstamped | length) unstamped\)"
    print $"staleness \(cargo xtask docs audit --stale\): ($report.checked_docs) checked, ($report.exempt_docs) exempt, ($report.untracked_docs) untracked"

    if not ($collisions | is-empty) {
        print $"\n(ansi red)COLLISIONS(ansi reset) — same topic in two homes; pick one canonical location:"
        for c in $collisions { print $"  ($c.base): ($c.paths | str join ' vs ') " }
    }

    if not ($dead | is-empty) {
        print $"\n(ansi red)DEAD LINKS(ansi reset):"
        for l in $dead { print $"  [($l.status)] ($l.url)\n      in: ($l.docs | str join ', ')" }
    }

    if ($stale | is-empty) {
        print $"\n(ansi green)No stale docs.(ansi reset) Every checked doc's sources are unchanged since its last substantive edit."
    } else {
        print $"\n(ansi yellow)STALE — source changed after the doc's last substantive edit(ansi reset):"
        for s in $stale {
            print $"  ($s.doc)  \(($s.reason)\)"
            for src in $s.sources { print $"      ($src.path): ($src.last_change_subject)" }
        }
    }

    if not ($unstamped | is-empty) {
        print $"\n(ansi yellow)UNSTAMPED(ansi reset) — no frontmatter, so freshness cannot be checked:"
        for u in $unstamped { print $"  ($u.path)" }
    }

    let failures = (($stale | length) + ($collisions | length) + ($dead | length))
    if $failures > 0 { exit 1 }
}
