#!/usr/bin/env nu

# Deterministic regression checks for documentation claims fixed by the MoA review.
def main [] {
    let fixture_path = ".claude/skills/docs/fixtures/moa-review.yaml"
    let fixture = (open $fixture_path)
    mut failures = []

    for check in $fixture.checks {
        if not ($check.path | path exists) {
            $failures = ($failures | append {
                id: $check.id
                path: $check.path
                reason: "document is missing"
            })
            continue
        }

        let text = (open --raw $check.path)
        for required in ($check.contains? | default []) {
            if not ($text | str contains $required) {
                $failures = ($failures | append {
                    id: $check.id
                    path: $check.path
                    reason: $"missing required text: ($required)"
                })
            }
        }
        for forbidden in ($check.excludes? | default []) {
            if ($text | str contains $forbidden) {
                $failures = ($failures | append {
                    id: $check.id
                    path: $check.path
                    reason: $"contains forbidden text: ($forbidden)"
                })
            }
        }
    }

    if ($failures | is-not-empty) {
        print ($failures | table)
        exit 1
    }

    print $"docs regression: ($fixture.checks | length) checks passed"
}
