---
status: archived
note: Never built; manual protocol tests used instead
---

> **ARCHIVED** — This document is not authoritative. See the current docs in the repo root.

# Macro: Protocol Test Utilities

**Date:** 2026-03-17
**Status:** Design
**Scope:** `minibox/src/protocol.rs` (test module), `miniboxd/tests/`

---

## Problem

`protocol.rs` has 29 encode/decode roundtrip tests. They share a structure that repeats
with minor variation — construct a value, encode, decode, match, assert fields:

```rust
#[test]
fn test_encode_decode_stop_request() {
    let req = DaemonRequest::Stop { id: "abc123".to_string() };
    let encoded = encode_request(&req).expect("encode failed");
    let decoded = decode_request(&encoded).expect("decode failed");
    match decoded {
        DaemonRequest::Stop { id } => assert_eq!(id, "abc123"),
        _ => panic!("wrong request type"),
    }
}
```

Each maestro Phase 1 command (`Architecture`, `Exec`, `Logs`, `Inspect`) adds 2–4 new
roundtrip tests. Without a macro, `protocol.rs` grows by ~20 lines per command variant,
and the `_ => panic!("wrong request type")` arm is a recurring source of test noise.

Additionally, `handler_tests.rs` has a repeated downcast-and-assert pattern used to inspect
mock state after handler invocations:

```rust
let mock_registry = deps.registry
    .as_any()
    .downcast_ref::<MockRegistry>()
    .expect("should be MockRegistry");
assert_eq!(mock_registry.pull_count(), 1);
```

This appears 5+ times and will grow with integration tests.

---

## Design

Two macros, both `macro_rules!`, both `#[cfg(test)]`-gated.

---

### Macro 1: `roundtrip!` (request)

````rust
/// Generate a `#[test]` that encodes a `DaemonRequest`, decodes it, pattern-matches
/// the result, and runs assertions on the destructured fields.
///
/// # Example
/// ```rust,ignore
/// roundtrip!(
///     test_encode_decode_stop,
///     DaemonRequest::Stop { id: "abc123".to_string() },
///     DaemonRequest::Stop { id } => {
///         assert_eq!(id, "abc123");
///     }
/// );
/// ```
macro_rules! roundtrip {
    (
        $test_name:ident,
        $value:expr,
        $pattern:pat => $assertions:block
    ) => {
        #[test]
        fn $test_name() {
            use crate::{encode_request, decode_request};
            let value = $value;
            let encoded = encode_request(&value).expect("encode failed");
            let decoded = decode_request(&encoded).expect("decode failed");
            match decoded {
                $pattern => $assertions,
                other => panic!("expected {}, got {:?}",
                    stringify!($pattern), other),
            }
        }
    };
}
````

**Response variant** (same pattern, different encode/decode functions):

```rust
macro_rules! roundtrip_resp {
    ($test_name:ident, $value:expr, $pattern:pat => $assertions:block) => {
        #[test]
        fn $test_name() {
            use crate::{encode_response, decode_response};
            let value = $value;
            let encoded = encode_response(&value).expect("encode failed");
            let decoded = decode_response(&encoded).expect("decode failed");
            match decoded {
                $pattern => $assertions,
                other => panic!("expected {}, got {:?}", stringify!($pattern), other),
            }
        }
    };
}
```

**Before (current `test_encode_decode_stop_request`):**

```rust
#[test]
fn test_encode_decode_stop_request() {
    let req = DaemonRequest::Stop { id: "abc123".to_string() };
    let encoded = encode_request(&req).expect("encode failed");
    let decoded = decode_request(&encoded).expect("decode failed");
    match decoded {
        DaemonRequest::Stop { id } => assert_eq!(id, "abc123"),
        _ => panic!("wrong request type"),
    }
}
```

**After:**

```rust
roundtrip!(
    test_encode_decode_stop,
    DaemonRequest::Stop { id: "abc123".to_string() },
    DaemonRequest::Stop { id } => { assert_eq!(id, "abc123"); }
);
```

5 lines → 5 lines, but the `_ => panic!` arm is now generated from `stringify!($pattern)`,
giving a useful message like `"expected DaemonRequest::Stop { id }, got ..."` rather than
`"wrong request type"`.

For a new maestro command, adding full roundtrip coverage is three lines:

```rust
roundtrip!(
    test_encode_decode_architecture,
    DaemonRequest::Architecture {},
    DaemonRequest::Architecture {} => {}
);
roundtrip!(
    test_encode_decode_exec,
    DaemonRequest::Exec { id: "c1".into(), command: "tmux".into(), args: vec!["ls".into()] },
    DaemonRequest::Exec { id, command, args } => {
        assert_eq!(id, "c1");
        assert_eq!(command, "tmux");
    }
);
```

---

### Macro 2: `mock!`

````rust
/// Downcast a `Dyn*` trait object to a mock type and assert a method's return value.
///
/// # Example
/// ```rust,ignore
/// mock!(deps.registry, MockRegistry, pull_count() == 1);
/// mock!(deps.registry, MockRegistry, last_pull_image() == Some("library/alpine".to_string()));
/// ```
macro_rules! mock {
    ($arc:expr, $concrete:ty, $method:ident() == $expected:expr) => {{
        let concrete = $arc
            .as_any()
            .downcast_ref::<$concrete>()
            .unwrap_or_else(|| panic!(
                "expected {}, got a different type",
                stringify!($concrete)
            ));
        assert_eq!(
            concrete.$method(),
            $expected,
            "{}.{}()",
            stringify!($concrete),
            stringify!($method)
        );
    }};
}
````

**Before:**

```rust
let mock_registry = deps.registry.as_any()
    .downcast_ref::<MockRegistry>()
    .expect("should be MockRegistry");
assert_eq!(mock_registry.pull_count(), 1);
```

**After:**

```rust
mock!(deps.registry, MockRegistry, pull_count() == 1);
```

---

## Placement

- `roundtrip!` and `roundtrip_resp!`: inside `protocol.rs` `#[cfg(test)]` module
- `mock!`: in `miniboxd/tests/common/mod.rs` (shared test utilities), or directly in handler_tests.rs if common/ doesn't exist yet

Both are `macro_rules!` without `#[macro_export]` — test-only, crate-internal.

## Why not property-based testing?

`proptest` or `quickcheck` could cover encode/decode correctness more thoroughly. However,
these macros serve a different goal: **documenting that specific field values round-trip
correctly** (e.g., `None` tags, Unicode in image names, u64::MAX limits). Property tests
complement but don't replace explicit value tests. Not a reason to avoid this macro.

---

## Migration

Existing tests can be migrated incrementally — the macro generates the same code they contain.
New protocol variants (maestro gaps) should use the macro from day one. No need to bulk-migrate
the 29 existing tests unless there's a cleanup pass underway.
