---
name: protocol-sites
description: >
  Use after adding a field to HandlerDependencies, DaemonRequest, or DaemonResponse — verifies
  all construction sites and adapter suites are updated together. Catches the common mistake of
  updating one adapter suite but not the other two.
disable-model-invocation: false
---

# protocol-sites — Handler Construction Site Check

After any change to `HandlerDependencies`, `DaemonRequest`, or `DaemonResponse`, verify all
construction and match sites are updated together.

## Step 1 — Check HandlerDependencies construction sites

```bash
nu scripts/check-protocol-sites.nu
```

Expected output (3 sites: native, gke, colima adapter suites):

```
check-protocol-sites: found 3 HandlerDependencies construction site(s) in
  crates/miniboxd/src/main.rs (expected 3)
OK: construction site count matches expected.
```

If count changed (e.g. you added a new adapter suite), update the expected value:

```bash
nu scripts/check-protocol-sites.nu --expected 4
```

## Step 2 — Check DaemonRequest match arms

When adding a new `DaemonRequest` variant, verify all match sites are updated:

```bash
# Find all match sites for DaemonRequest
rg "DaemonRequest::" crates/ --type rust -l
```

Every file that matches must handle the new variant. Missing arms cause a compiler error, but
check now rather than waiting for `cargo check`.

## Step 3 — Check DaemonResponse is_terminal_response

When adding a new `DaemonResponse` variant, update `is_terminal_response()` in `server.rs`:

```bash
rg "is_terminal_response" crates/daemonbox/src/server.rs
```

Non-terminal variants (like `ContainerOutput`) must be explicitly listed. All others default
to terminal. Missing the new variant causes the streaming connection to close prematurely.

## Step 4 — Verify backward compatibility

New fields on existing variants must use `#[serde(default)]`:

```rust
// Good — existing clients that omit the field continue to work
#[serde(default)]
pub new_field: bool,
```

Check with:

```bash
rg "serde.*default" crates/minibox-core/src/protocol.rs
```

## Step 5 — Run cargo check

```bash
cargo check --workspace
```

Must pass with zero errors before committing protocol changes.

## Key Rules

- **All 3 adapter suites must be updated together** — native, gke, colima in `miniboxd/src/main.rs`
- **`is_terminal_response()` must be updated for new response variants** — see `server.rs`
- **New fields need `#[serde(default)]`** — backward compatibility with existing clients
- **Protocol types live in `minibox-core/src/protocol.rs`** — canonical source; `minibox` re-exports
