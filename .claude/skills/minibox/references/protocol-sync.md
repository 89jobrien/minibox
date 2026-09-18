---
name: protocol-sync
description:
  After adding or modifying a DaemonRequest variant or field, audit the canonical
  protocol and every server, frontend, and test consumer for consistency.
  Invoke after any protocol.rs change.
disable-model-invocation: true
---

# protocol-sync — DaemonRequest propagation audit

Run this skill after any change to `DaemonRequest` in the canonical protocol. It walks
the dispatch, handler, frontend, and test consumers and produces a status table.

---

## Step 1 — Extract the canonical variant list

Read `crates/minibox-core/src/protocol.rs` and collect every `DaemonRequest` variant name.
There is no second CLI-local protocol definition; all frontends use this canonical type.

Use the Grep tool:

```
pattern: "^\s+\w+\s*\{" or enum variant lines inside DaemonRequest
file: crates/minibox-core/src/protocol.rs
```

Read the surrounding enum body and list all variants and fields. Treat it as the
single wire-contract inventory for the remaining checks.

---

## Step 2 — Check server.rs dispatch match

The dispatch match in `crates/minibox/src/daemon/server.rs` must have an arm for every
`DaemonRequest` variant.

Grep `crates/minibox/src/daemon/server.rs` for `DaemonRequest::` to list all handled
variants. Compare against the variant list from Step 1. Report any variant that
appears in protocol.rs but is absent from the match.

Also inspect `DaemonResponse::is_terminal()` in the canonical protocol when response
flow changes.

---

## Step 3 — Check split handler modules

Each implemented operation should have a corresponding handler function under
`crates/minibox/src/daemon/handler/`. The naming convention is `handle_<snake_case>`.

Search `crates/minibox/src/daemon/handler/` for `fn handle_` to list all handler
functions. For each variant from Step 1, check whether a corresponding `handle_`
function exists. Report missing handlers.

For `Run`, trace the server dispatch into `RunParams` and the preparation/request
modules rather than relying on a fixed historical function chain.

---

## Step 4 — Check CLI parser

The CLI constructs `DaemonRequest` variants when sending commands to the daemon.

Search `crates/mbx/src/main.rs` and `crates/mbx/src/commands/` for
`DaemonRequest::` to find all construction sites. For each variant from Step 1,
confirm at least one construction site exists. Report variants with no CLI entry
point (they may be intentionally daemon-internal, so note them as "no CLI path"
rather than "MISSING" unless context suggests otherwise). Repeat for MCP, Crux,
and TUI when the operation is exposed by those frontends.

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
| Variant | protocol.rs | server.rs | handler/ | mbx | integrations | tests |
|---------|-------------|-----------|----------|-----|--------------|-------|
| Run | OK | OK | OK | OK | MCP/Crux | 12 tests |
| Stop | OK | OK | OK | OK | MCP/Crux | 4 tests |
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

**Missing from server.rs dispatch**: Add a match arm in the `handle_request` (or
equivalent) function. Follow the existing pattern for similar variants.

**Missing handler function**: Add `async fn handle_<variant>(...)` to the owning handler module.
If it involves container operations, wrap blocking work in
`tokio::task::spawn_blocking`.

**Missing from CLI**: Add a subcommand in `crates/mbx/src/main.rs` or a new
file under `crates/mbx/src/commands/`. Wire it to construct the
`DaemonRequest` variant and send via `DaemonClient`.

**Missing tests**: Add at least one happy-path and one error-path test in
`crates/minibox/tests/daemon_handler_failure_tests.rs`. Use `create_test_deps_with_dir` and
`handle_run_once()` helpers already defined in that file.

---

## Quick-check for new field additions (not new variants)

If a field was added to an existing variant (not a new variant), do the following
instead of the full audit:

1. Confirm the canonical variant has the new field and uses `#[serde(default)]`
   when wire compatibility requires it.
2. Search server dispatch, handlers, and all frontend construction sites.
3. Search tests for all construction sites of that variant — confirm they
   compile (run `cargo check -p minibox` to verify).
4. Confirm frontend construction sites set the
   new field or rely on `Default`.

## Dashbox Logging

After completing the audit, append to `$HOME/.mbx/automation-runs.jsonl` so this run appears
in the automation-runs log:

```bash
echo '{"run_id":"'$(date -u +%Y-%m-%dT%H:%M:%S)'","script":"protocol-sync","status":"complete","duration_s":0,"output":"Audited N variants: M gaps found"}' >> ~/.mbx/automation-runs.jsonl
```

Replace `N` with the variant count and `M` with the number of gaps found.
