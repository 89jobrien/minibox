---
name: protocol-sync
description:
  After adding or modifying a DaemonRequest variant or field, audit all
  6 propagation sites for consistency. Reports missing or mismatched implementations.
  Invoke after any protocol.rs change.
disable-model-invocation: true
---

# protocol-sync — DaemonRequest propagation audit

Run this skill after any change to `DaemonRequest` in either `protocol.rs`. It walks
all 6 propagation sites and produces a status table.

---

## Step 1 — Extract variant lists from both protocol.rs files

Read both files and collect every `DaemonRequest` variant name.

- `crates/minibox-core/src/protocol.rs` — cross-platform definition
- `crates/mbx/src/protocol.rs` — mbx-local definition (must stay in sync)

Use the Grep tool:

```
pattern: "^\s+\w+\s*\{" or enum variant lines inside DaemonRequest
files: crates/minibox-core/src/protocol.rs, crates/mbx/src/protocol.rs
```

Concretely: grep for `DaemonRequest` in both files, then read the surrounding enum
body. List all variant names from each file. If the two lists differ, flag every
discrepancy immediately — this is the most critical gap.

---

## Step 2 — Check server.rs dispatch match

The dispatch match in `crates/minibox/src/daemon/server.rs` must have an arm for every
`DaemonRequest` variant.

Grep `crates/minibox/src/daemon/server.rs` for `DaemonRequest::` to list all handled
variants. Compare against the variant list from Step 1. Report any variant that
appears in protocol.rs but is absent from the match.

Note: per CLAUDE.md, adding a variant also requires updating `is_terminal_response()`
in the same file if the new variant is non-terminal (like `ContainerOutput`).

---

## Step 3 — Check handler.rs functions

Each `DaemonRequest` variant should have a corresponding handler function in
`crates/minibox/src/daemon/handler.rs`. The naming convention is `handle_<snake_case>`.

Grep `crates/minibox/src/daemon/handler.rs` for `fn handle_` to list all handler
functions. For each variant from Step 1, check whether a corresponding `handle_`
function exists. Report missing handlers.

Also check that the `handle_run` parameter chain is consistent: per CLAUDE.md,
adding a parameter requires updating these sites in order:

1. server.rs dispatch pattern match
2. `handle_run`
3. `handle_run_streaming`
4. `run_inner_capture`
5. `run_inner`

---

## Step 4 — Check CLI parser

The CLI constructs `DaemonRequest` variants when sending commands to the daemon.

Grep `crates/minibox-cli/src/main.rs` and `crates/minibox-cli/src/commands/` for
`DaemonRequest::` to find all construction sites. For each variant from Step 1,
confirm at least one construction site exists. Report variants with no CLI entry
point (they may be intentionally daemon-internal, so note them as "no CLI path"
rather than "MISSING" unless context suggests otherwise).

---

## Step 5 — Check test construction sites

Grep `crates/minibox/tests/daemon_handler_failure_tests.rs` for `DaemonRequest::` to list all
variant construction sites in tests. For each variant from Step 1, count how many
test cases construct it. A count of 0 is a coverage gap — flag it.

Also grep the broader workspace for any other test files that construct `DaemonRequest`:

```
pattern: DaemonRequest::
path: crates/
glob: **/*test*.rs
```

---

## Step 6 — Report

Output a markdown table with one row per `DaemonRequest` variant:

```
| Variant | core/protocol.rs | mbx/protocol.rs | server.rs | handler.rs | CLI | tests |
|---------|-----------------|-----------------|-----------|------------|-----|-------|
| RunContainer | OK | OK | OK | OK | OK | 12 tests |
| StopContainer | OK | OK | OK | OK | OK | 4 tests |
| ... | ... | ... | ... | ... | ... | ... |
```

Cell values:

- `OK` — present and accounted for
- `MISSING` — absent from this site
- `N tests` — for the tests column, show the count
- `no CLI path` — variant exists but has no CLI construction (may be intentional)

---

## Remediation guidance

If gaps are found, provide exact instructions:

**Missing from mbx/protocol.rs**: Add the variant definition, matching the struct
fields exactly from minibox-core/protocol.rs. Add `#[serde(default)]` to any new
fields for backward compatibility.

**Missing from server.rs dispatch**: Add a match arm in the `handle_request` (or
equivalent) function. Follow the existing pattern for similar variants.

**Missing handler function**: Add `async fn handle_<variant>(...)` to handler.rs.
If it involves container operations, wrap blocking work in
`tokio::task::spawn_blocking`.

**Missing from CLI**: Add a subcommand in `crates/minibox-cli/src/main.rs` or a new
file under `crates/minibox-cli/src/commands/`. Wire it to construct the
`DaemonRequest` variant and send via `DaemonClient`.

**Missing tests**: Add at least one happy-path and one error-path test in
`crates/minibox/tests/daemon_handler_failure_tests.rs`. Use `create_test_deps_with_dir` and
`handle_run_once()` helpers already defined in that file.

---

## Quick-check for new field additions (not new variants)

If a field was added to an existing variant (not a new variant), do the following
instead of the full audit:

1. Grep both protocol.rs files for the variant struct — confirm both have the new
   field.
2. Confirm the new field has `#[serde(default)]`.
3. Grep handler_tests.rs for all construction sites of that variant — confirm they
   compile (run `cargo check -p minibox` to verify).
4. Grep CLI source for construction sites of that variant — confirm they set the
   new field or rely on `Default`.

## Dashbox Logging

After completing the audit, append to `~/.mbx/automation-runs.jsonl` so this run appears
in the automation-runs log:

```bash
echo '{"run_id":"'$(date -u +%Y-%m-%dT%H:%M:%S)'","script":"protocol-sync","status":"complete","duration_s":0,"output":"Audited N variants: M gaps found"}' >> ~/.mbx/automation-runs.jsonl
```

Replace `N` with the variant count and `M` with the number of gaps found.
