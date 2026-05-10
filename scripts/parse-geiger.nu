#!/usr/bin/env nu
# parse-geiger.nu — parse cargo-geiger report into structured records
#
# Usage:
#   nu scripts/parse-geiger.nu geiger-report.txt
#   nu scripts/parse-geiger.nu geiger-report.txt | to json
#   nu scripts/parse-geiger.nu geiger-report.txt | where status == "unsafe" | sort-by expressions_used -r

# Columns in the report: Functions  Expressions  Impls  Traits  Methods  Dependency
# Status symbols: !  = unsafe used, :) = forbids_unsafe, ? = no_forbid

def parse-ratio [s: string] {
    let parts = ($s | str trim | split row '/')
    if ($parts | length) == 2 {
        {used: ($parts | first | into int), total: ($parts | last | into int)}
    } else {
        {used: 0, total: 0}
    }
}

def parse-status [sym: string] {
    let s = ($sym | str trim)
    match $s {
        "!" => "unsafe",
        ":)" => "forbids_unsafe",
        _ => "no_forbid",
    }
}

# Regex for data lines (after ANSI stripping):
# five x/y groups, a status symbol, then the dependency tree + crate name
def data-pattern [] {
    "^([0-9]+/[0-9]+)[ ]+([0-9]+/[0-9]+)[ ]+([0-9]+/[0-9]+)[ ]+([0-9]+/[0-9]+)[ ]+([0-9]+/[0-9]+)[ ]+([!?]|:[)]) +(.+)$"
}

def main [report: path] {
    let pat = data-pattern

    open --raw $report
        | lines
        | each { |line| $line | ansi strip }
        | where { |line| ($line | parse --regex $pat | length) > 0 }
        | each { |line|
            let m = ($line | parse --regex $pat | first)

            # Strip tree box-drawing chars (├ └ │ ─) and collapse whitespace
            let dep_raw = ($m.capture6 | str trim)
            let dep_clean = ($dep_raw
                | str replace --all --regex '[│├└─]' ' '
                | str replace --all --regex '  +' ' '
                | str trim)

            # Last token is version, rest is crate name
            let parts = ($dep_clean | split row ' ' | where { |p| $p != '' })
            let n = ($parts | length)
            let version = if $n >= 2 { $parts | last } else { "" }
            let name = if $n >= 2 {
                $parts | slice 0..($n - 2) | str join ' '
            } else {
                $dep_clean
            }

            let fns     = parse-ratio $m.capture0
            let exprs   = parse-ratio $m.capture1
            let impls   = parse-ratio $m.capture2
            let traits  = parse-ratio $m.capture3
            let methods = parse-ratio $m.capture4

            {
                name:               $name
                version:            $version
                status:             (parse-status $m.capture5)
                functions_used:     $fns.used
                functions_total:    $fns.total
                expressions_used:   $exprs.used
                expressions_total:  $exprs.total
                impls_used:         $impls.used
                impls_total:        $impls.total
                traits_used:        $traits.used
                traits_total:       $traits.total
                methods_used:       $methods.used
                methods_total:      $methods.total
            }
        }
}
