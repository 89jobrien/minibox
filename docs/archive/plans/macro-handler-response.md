---
status: archived
note: Never built; manual match/wrap is the pattern
---

> **ARCHIVED** — This document is not authoritative. See the current docs in the repo root.

# Macro: Handler Response Wrapping

**Date:** 2026-03-17
**Status:** Design
**Scope:** `miniboxd/src/handler.rs`

---

## Problem

Every handler in `handler.rs` wraps an inner async function with identical error-handling boilerplate:

```rust
pub async fn handle_stop(id: String, state: Arc<DaemonState>) -> DaemonResponse {
    match stop_inner(&id, &state).await {
        Ok(()) => DaemonResponse::Success {
            message: format!("container {id} stopped"),
        },
        Err(e) => {
            error!("handle_stop error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}
```

The `Err` branch is **identical in structure** across all five current handlers
(`handle_run`, `handle_stop`, `handle_remove`, `handle_pull`, and `handle_list` implicitly).
Each maestro integration gap (exec, logs, architecture, named-container ops) adds another handler.
Without a macro, every new handler will duplicate the error path.

---

## Design

A single `macro_rules!` macro in `handler.rs` for the error-to-response translation:

````rust
/// Map a `Result<_, E>` to a `DaemonResponse`.
///
/// On `Ok(value)`, evaluates `$ok_expr` (the `value` binding is in scope).
/// On `Err(e)`, logs with `error!` and returns `DaemonResponse::Error`.
///
/// # Example
/// ```rust,ignore
/// handle!(stop_inner(&id, &state).await, {
///     DaemonResponse::Success { message: format!("container {id} stopped") }
/// })
///
macro_rules! handle {
    ($result:expr, $ok_expr:expr) => {
        match $result {
            Ok(_) => $ok_expr,
            Err(e) => {
                // $crate doesn't work inside the same crate; use tracing directly
                tracing::error!("{e:#}");
                DaemonResponse::Error {
                    message: format!("{e:#}"),
                }
            }
        }
    };
    // Variant that binds the Ok value
    ($result:expr, |$ok:ident| $ok_expr:expr) => {
        match $result {
            Ok($ok) => $ok_expr,
            Err(e) => {
                tracing::error!("{e:#}");
                DaemonResponse::Error {
                    message: format!("{e:#}"),
                }
            }
        }
    };
}
````

### Before / After

**Before (`handle_stop`):**

```rust
pub async fn handle_stop(id: String, state: Arc<DaemonState>) -> DaemonResponse {
    match stop_inner(&id, &state).await {
        Ok(()) => DaemonResponse::Success {
            message: format!("container {id} stopped"),
        },
        Err(e) => {
            error!("handle_stop error: {e:#}");
            DaemonResponse::Error { message: format!("{e:#}") }
        }
    }
}
```

**After:**

```rust
pub async fn handle_stop(id: String, state: Arc<DaemonState>) -> DaemonResponse {
    handle!(stop_inner(&id, &state).await, {
        DaemonResponse::Success { message: format!("container {id} stopped") }
    })
}
```

**Before (`handle_run`):**

```rust
pub async fn handle_run(...) -> DaemonResponse {
    match run_inner(...).await {
        Ok(id) => DaemonResponse::ContainerCreated { id },
        Err(e) => {
            error!("handle_run error: {e:#}");
            DaemonResponse::Error { message: format!("{e:#}") }
        }
    }
}
```

**After:**

```rust
pub async fn handle_run(...) -> DaemonResponse {
    handle!(run_inner(...).await, |id| DaemonResponse::ContainerCreated { id })
}
```

### New handler for maestro (example: `Architecture`)

```rust
pub async fn handle_architecture() -> DaemonResponse {
    handle!(architecture_inner().await, |arch| {
        DaemonResponse::Architecture { value: arch }
    })
}
```

One line for the entire error path. The handler reads as the happy path only.

---

## Placement

`macro_rules!` defined at the top of `handler.rs` (module-private, no `#[macro_export]` needed).
Not exported — it's only meaningful within handler context.

## Scope limits

This macro handles the **synchronous response** pattern only. Handlers that need to return
a streaming response (exec, logs — see `macro-streaming-protocol.md`) will use a different
mechanism. Do not extend this macro to cover streaming cases; keep the two patterns separate.

---

## Maestro Connection

Every Phase 1 maestro gap (named containers, architecture, logs-as-string, exec-as-output)
adds a handler. Each handler's error path is identical. The macro makes those additions
a mechanical one-liner rather than a copy-paste exercise.

Phase 1 additions that use this macro:

- `handle_architecture` — `Ok(String)` → `DaemonResponse::Architecture { value }`
- `handle_exec` (non-streaming) — `Ok(ExecOutput)` → `DaemonResponse::ExecResult { ... }`
- `handle_logs` — `Ok(String)` → `DaemonResponse::Logs { output }`
- `handle_inspect` — `Ok(ContainerInfo)` → `DaemonResponse::ContainerDetail { ... }`
