---
status: archived
note: Never built; hand-written dispatch in daemonbox/handler.rs is the real approach
---

# Macro: Command Registration (Protocol + Dispatch)

**Date:** 2026-03-17
**Status:** Design
**Scope:** `linuxbox/src/protocol.rs`, `miniboxd/src/server.rs`, `miniboxd/src/handler.rs`

---

## Problem

Adding a new daemon command today requires touching **four locations**:

1. `protocol.rs` — add variant to `DaemonRequest` enum
2. `protocol.rs` — add variant to `DaemonResponse` enum (one or more)
3. `server.rs` — add match arm to `dispatch()`
4. `handler.rs` — write `handle_X()` and `X_inner()` functions

Each maestro Phase 1 gap adds one command:

- `Architecture` — query host arch
- `Exec` — run command in container, return output
- `Logs` — return captured stdout/stderr
- Named container lookup for `exists` / `is_running`

Without structure, protocol growth becomes error-prone (missing match arms cause
non-exhaustive compile errors in `dispatch`, but missing handler wiring produces
silent wrong-response bugs during protocol evolution).

---

## Design Goals

1. Single authoritative location where a command's input fields, response type, and handler binding are declared together
2. The compiler enforces completeness — unregistered commands don't compile
3. Minimal magic — the generated code looks like what a developer would write by hand
4. Compatible with the existing hexagonal architecture (handlers still receive `Arc<DaemonState>` and `Arc<HandlerDependencies>`)

---

## Approach Comparison

### Option A: `macro_rules!` dispatch table (recommended)

A macro that generates only the `dispatch()` match arms from a registration list.
The protocol enum variants and handler functions remain hand-written (they vary too much in
structure to template usefully).

The macro captures the mapping in one place — if you add a command to the registration
list but forget the handler, the compile error is "unresolved function `handler::handle_X`"
rather than a silent bug.

```rust
// In server.rs
macro_rules! route {
    (
        $request:expr, $state:expr, $deps:expr;
        $( $variant:ident { $($field:ident),* } => $handler:expr ),+ $(,)?
    ) => {
        match $request {
            $(
                DaemonRequest::$variant { $($field),* } =>
                    $handler($($field,)* Arc::clone(&$state), Arc::clone(&$deps)).await,
            )+
        }
    };
    // Unit variants (no fields)
    (
        $request:expr, $state:expr, $deps:expr;
        $( $variant:ident { $($field:ident),* } => $handler:expr ),+;
        unit: $( $unit_variant:ident => $unit_handler:expr ),+ $(,)?
    ) => { ... }
}
```

**Usage:**

```rust
async fn dispatch(request: DaemonRequest, state: Arc<DaemonState>, deps: Arc<HandlerDependencies>) -> DaemonResponse {
    route!(request, state, deps;
        Run    { image, tag, command, memory_limit_bytes, cpu_weight } => handler::handle_run,
        Stop   { id }                                                  => handler::handle_stop,
        Remove { id }                                                  => handler::handle_remove,
        Pull   { image, tag }                                          => handler::handle_pull,
        List   {}                                                      => handler::handle_list,
        // Adding a maestro gap is one line:
        Architecture {}                                                => handler::handle_architecture,
        Exec   { id, command, args }                                   => handler::handle_exec,
        Logs   { id }                                                  => handler::handle_logs,
    )
}
```

Adding a command = one line here + one variant in `DaemonRequest` + one handler function.
The macro ensures the dispatch table and the enum stay synchronized at compile time.

### Option B: Proc macro `#[command]` derive

A proc macro attribute on the enum that generates dispatch boilerplate:

```rust
#[command(handler = "handler::handle_run")]
Run { image: String, tag: Option<String>, ... }
```

Pros: truly single-location definition, handler binding right next to the variant.
Cons: requires a separate `minibox-macros` crate, `syn`/`quote` dependencies, significantly
more complex to write and maintain. Overkill for the current scale (8–10 commands total).

### Option C: Hand-written (status quo)

Pros: transparent, no macro overhead.
Cons: the dispatch function grows unboundedly; adding a command requires touching two files
and it's easy to mis-wire field names in the destructuring.

---

## Recommendation: Option A

The `route!` macro captures the only genuinely repetitive structure
(variant → destructure fields → call handler → await) without hiding the protocol definition
or handler logic. It stays as a `macro_rules!` in `server.rs` — no new crates, no proc macro
complexity. When the command count grows beyond ~15, revisit Option B.

---

## Implementation Notes

### Field forwarding in `macro_rules!`

`macro_rules!` cannot call a function with a dynamic number of arguments templated from
matched fields. The macro above works by destructuring the variant into named bindings and
then calling the handler with those bindings in order. **This requires handler argument order
to match the enum field order.** Document this as a convention.

Alternatively, bundle all request fields into a request struct (e.g., `RunRequest`) and pass
that instead of individual fields — this removes the ordering coupling:

```rust
// handler signature becomes:
pub async fn handle_run(req: RunRequest, state: Arc<DaemonState>, deps: Arc<HandlerDependencies>) -> DaemonResponse
```

This is a bigger refactor but makes the macro simpler and handler signatures more uniform.
Worth doing alongside adding the first maestro command if the team finds field ordering fragile.

### `List` (unit variant)

`DaemonRequest::List` has no fields. The macro needs a fallback rule or handle this explicitly.
Simplest: give it a struct variant with no fields `List {}` — serde handles this correctly
(empty JSON object `{}`), and the macro pattern `List {}` matches cleanly.

---

## Maestro Phase 1 Commands to Register

```
Architecture {}                        → handle_architecture
Exec   { id, command, args }           → handle_exec          (non-streaming, returns ExecResult)
Logs   { id }                          → handle_logs
Inspect { name_or_id }                 → handle_inspect       (named container lookup)
```

The `Run` variant gains a `name: Option<String>` field for named container support — backward
compatible since `Option` defaults to `None` in serde.

---

## Related Docs

- `macro-handler-response.md` — the error-wrapping macro used inside each handler
- `macro-streaming-protocol.md` — for exec/TTY streaming (changes dispatch from request-response to streaming, making this dispatch table macro insufficient for Phase 2)
