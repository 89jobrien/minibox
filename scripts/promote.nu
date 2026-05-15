#!/usr/bin/env nu
# promote.nu — cascade-merge through the stability pipeline
#
# Usage:
#   nu scripts/promote.nu                     # develop → next → staging → main
#   nu scripts/promote.nu --from next         # next → staging → main
#   nu scripts/promote.nu --from next --to staging  # next → staging only
#
# The script:
#   1. Stashes dirty handoff files (.ctx/) so branch switches don't fail
#   2. Fast-forward merges each tier in sequence
#   3. Pushes all promoted branches in one call
#   4. Returns to the original branch and restores the stash

def main [
    --from: string = "develop"   # starting branch to promote from
    --to: string = "main"        # furthest branch to promote to
    --dry-run                    # print plan without executing
] {
    let pipeline = ["develop", "next", "staging", "main"]

    let from_idx = $pipeline | enumerate | where item == $from | get index | first
    let to_idx   = $pipeline | enumerate | where item == $to   | get index | first

    if $from_idx >= $to_idx {
        error make { msg: $"--from ($from) must be earlier in pipeline than --to ($to)" }
    }

    let tiers = $pipeline | slice $from_idx..$to_idx

    # pairs: (source, dest) for each merge step
    let steps = $tiers | window 2 | each { |w| { src: $w.0, dst: $w.1 } }
    let targets = $tiers | slice 1.. # branches that will be pushed

    print $"Promote: ($tiers | str join ' → ')"

    if $dry_run {
        for s in $steps { print $"  merge ($s.src) → ($s.dst)" }
        print $"  push ($targets | str join ' ')"
        return
    }

    let origin = git branch --show-current | str trim

    # stash dirty handoff files only (non-blocking if nothing to stash)
    let dirty = git status --short | str trim
    let has_ctx = ($dirty | str contains ".ctx/")
    if $has_ctx {
        git stash push -m "promote-wip" .ctx/HANDOFF.md .ctx/HANDOFF.minibox.minibox.yaml
    }

    # cascade merges
    for s in $steps {
        git checkout $s.dst
        git merge $s.src -m $"Merge branch '($s.src)' into ($s.dst)"
    }

    # push all targets at once
    git push origin ...$targets

    # return to original branch
    git checkout $origin

    if $has_ctx {
        git stash pop
    }

    print $"Done. All branches at (git rev-parse --short HEAD | str trim)."
}
