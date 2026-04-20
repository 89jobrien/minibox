#!/usr/bin/env nu
# check-protocol-sites.nu
#
# CI guard: count HandlerDependencies construction sites in miniboxd/src/main.rs
# and warn if the count deviates from the expected value (currently 3: native,
# gke, colima adapter suites).
#
# Usage:
#   nu scripts/check-protocol-sites.nu
#   nu scripts/check-protocol-sites.nu --expected 4   # override expected count
#
# Exit codes:
#   0 — count matches expected (or --warn-only is set)
#   1 — count mismatch and --warn-only is not set

def main [
    --expected: int = 3          # expected number of HandlerDependencies { ... } sites
    --warn-only                  # print a warning instead of exiting 1 on mismatch
    --file: string = "crates/miniboxd/src/main.rs"  # file to search
] {
    let pattern = "HandlerDependencies \\{"

    # Collect matching lines (rg exits 1 when no matches — that's not an error here)
    let result = (do { ^rg --line-number $pattern $file } | complete)
    let matches = if $result.exit_code == 0 {
        $result.stdout | lines | where { |l| ($l | str trim) != "" }
    } else {
        []
    }

    let count = ($matches | length)

    print $"check-protocol-sites: found ($count) HandlerDependencies construction site\(s\) in ($file) \(expected ($expected)\)"

    if $count != $expected {
        let msg = $"WARN: HandlerDependencies construction site count changed: expected ($expected), got ($count). Update all adapter suites together."
        print $msg

        for line in $matches {
            print $"  ($line)"
        }

        if not $warn_only {
            print "Failing due to count mismatch. Pass --warn-only to suppress exit 1."
            exit 1
        }
    } else {
        print "OK: construction site count matches expected."
        for line in $matches {
            print $"  ($line)"
        }
    }
}
