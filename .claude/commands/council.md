---
name: council
description: >
  Multi-role AI code review of the current branch. Runs 3 roles (core) or 5 roles
  (extensive) and synthesises findings. Use before merging or when a thorough review
  is needed.
argument-hint: "[--base <branch>] [--mode core|extensive] [--no-synthesis]"
---

# council

Runs multiple reviewer roles against the diff and synthesises findings into a single report.

```nu
nu scripts/council.nu                           # core (3 roles) vs main
nu scripts/council.nu --mode extensive          # 5 roles vs main
nu scripts/council.nu --base develop            # diff vs develop
nu scripts/council.nu --mode extensive --no-synthesis  # skip synthesis step
```

Modes: `core` (3 roles, faster), `extensive` (5 roles, thorough).
