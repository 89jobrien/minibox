#!/bin/bash
# Classify each stash by evidence:
#  - base_reachable: is the stash's base commit still an ancestor of develop?
#  - landed: does every file the stash touches already have identical content in develop?
# Read-only. Never drops, applies, or mutates anything.
set -uo pipefail
cd /Users/joe/dev/minibox

REF=develop
printf '%-9s %-6s %-6s %-6s %-5s %s\n' STASH BASE FILES LANDED FILESET LABEL
printf '%.0s-' {1..100}
echo

n=$(git stash list | wc -l | tr -d ' ')
for i in $(seq 0 $((n - 1))); do
    s="stash@{$i}"
    base=$(git rev-parse "$s^1" 2>/dev/null || echo "-")

    if [ "$base" != "-" ] && git merge-base --is-ancestor "$base" "$REF" 2>/dev/null; then
        base_ok="yes"
    else
        base_ok="NO"
    fi

    files=$(git stash show --name-only "$s" 2>/dev/null)
    total=0
    landed=0
    missing=0
    differing=0
    fileset="-"
    for f in $files; do
        total=$((total + 1))
        [ -z "$fileset" ] && fileset="$f"
        if git cat-file -e "$REF:$f" 2>/dev/null; then
            a=$(git rev-parse "$s:$f" 2>/dev/null || echo none)
            b=$(git rev-parse "$REF:$f" 2>/dev/null || echo none)
            if [ "$a" = "$b" ]; then landed=$((landed + 1)); else differing=$((differing + 1)); fi
        else
            missing=$((missing + 1))
        fi
    done

    if [ "$total" -eq 0 ]; then
        verdict="no-files"
    elif [ "$landed" -eq "$total" ]; then
        verdict="landed"
    elif [ "$missing" -gt 0 ]; then
        verdict="orphaned"
    else
        verdict="partial"
    fi

    label=$(git stash list | sed -n "$((i + 1))p" | cut -d: -f2- | sed 's/^ *//')
    printf '%-9s %-6s %-6s %-6s %-5s %s\n' "$s" "$base_ok" "$total" "$landed" "$verdict" "$label"
done

echo
echo "detail (verdict per stash, with the non-landed files):"
for i in $(seq 0 $((n - 1))); do
    s="stash@{$i}"
    out=""
    for f in $(git stash show --name-only "$s" 2>/dev/null); do
        if ! git cat-file -e "$REF:$f" 2>/dev/null; then
            out="$out MISSING:$f"
        else
            a=$(git rev-parse "$s:$f" 2>/dev/null || echo none)
            b=$(git rev-parse "$REF:$f" 2>/dev/null || echo none)
            [ "$a" != "$b" ] && out="$out DIFFERS:$f"
        fi
    done
    [ -z "$out" ] && out=" (all content present in $REF)"
    printf '  %-10s%s\n' "$s" "$out"
done
