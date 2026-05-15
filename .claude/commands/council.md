---
name: council
description: >
  Multi-role AI code review of the current branch. Runs 3 roles (core) or 5 roles
  (extensive) and synthesises findings. Use before merging or when a thorough review
  is needed.
argument-hint: "[--base <branch>] [--mode core|extensive] [--no-synthesis]"
agent: atelier:sentinel
allowed-tools: [Bash, Read]
---

Multi-role review of the diff between the current branch and `--base` (default: `main`).

Parse `$ARGUMENTS` for: `--base <branch>`, `--mode core|extensive`, `--no-synthesis`.

**Roles:**

- **core** (default, 3 roles): Security, Correctness, Architecture
- **extensive** (5 roles): adds Performance and Maintainability

**For each role:**

1. Run `git diff <base>...HEAD` to get the diff
2. Apply the role's lens to the diff — produce findings with severity tags:
   `[BLOCKING]`, `[WARN]`, `[NIT]`
3. Each finding includes: file:line, description, suggested fix

**Security role checklist:**
- Path traversal (user input to Path::join without validate_layer_path)
- `.unwrap()` in non-test code
- `fork`/`clone` inside `async fn` without spawn_blocking
- `println!`/`eprintln!` in daemon code
- New serde fields without `#[serde(default)]`
- `unsafe` blocks without SAFETY comments

**Synthesis** (unless `--no-synthesis`):
After all roles complete, produce a single ranked list: all BLOCKING items first,
then WARNs, then NITs. Deduplicate overlapping findings across roles.

Modes: `core` (3 roles, faster), `extensive` (5 roles, thorough).
