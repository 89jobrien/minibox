---
name: promote
description: >
  Cascade-merge through the stability pipeline: develop → next → staging → main.
  Use after CI is green on a branch and you want to advance it through the pipeline.
argument-hint: "[--from <branch>] [--to <branch>] [--dry-run]"
---

# promote

Fast-forward merges each tier in sequence, pushes all promoted branches, and returns
to the original branch. Stashes dirty `.ctx/` handoff files across branch switches.

```nu
nu scripts/promote.nu                          # develop → next → staging → main
nu scripts/promote.nu --from next              # next → staging → main
nu scripts/promote.nu --from next --to staging # next → staging only
nu scripts/promote.nu --dry-run                # print plan without executing
```

Pipeline order: `develop` → `next` → `staging` → `main`

**Rules:**
- Do not promote `next` → `staging` without confirming `next` CI is green.
- Do not promote `staging` → `main` without confirming `staging` CI is green.
- Use `--dry-run` first to verify the plan before executing.
