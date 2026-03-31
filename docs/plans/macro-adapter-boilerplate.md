---
status: archived
completed: "2026-03-17"
note: adapt!/as_any! shipped in minibox-macros; design predates actual structure
---

# Macro: Adapter Boilerplate Elimination

**Date:** 2026-03-17
**Status:** Design
**Scope:** `mbx/src/adapters/`, `mbx/src/domain.rs`

---

## Problem

Every adapter implementation carries two structurally identical, zero-variation boilerplate blocks.

### `impl AsAny` — 21 occurrences

```rust
impl AsAny for CgroupV2Limiter {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

Found in: `limiter.rs`, `filesystem.rs`, `runtime.rs`, `registry.rs`, `mocks.rs` (×4),
`colima.rs` (×4), `gke.rs` (×3), `wsl.rs` (×3), `docker_desktop.rs` (×3).

### `impl Default` delegating to `new()` — 13 occurrences

```rust
impl Default for CgroupV2Limiter {
    fn default() -> Self {
        Self::new()
    }
}
```

Found in: all adapter structs that have a no-argument `new()`.

Adding a new adapter currently requires writing both blocks per struct. With the maestro
integration plan, 2–3 new adapter structs are expected (e.g., `MiniboxContainerProvider`).

---

## Design

Two `macro_rules!` macros in `mbx/src/adapters/mod.rs`, exported from the crate root.

### Macro 1: `as_any!`

````rust
/// Implement `AsAny` for one or more types.
///
/// # Example
/// ```rust
/// as_any!(CgroupV2Limiter, OverlayFilesystem);
/// ```
#[macro_export]
macro_rules! as_any {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::domain::AsAny for $t {
                fn as_any(&self) -> &dyn ::std::any::Any {
                    self
                }
            }
        )+
    };
}
````

Accepts a comma-separated list so all structs in one adapter file can be covered in one call.

### Macro 2: `default_new!`

````rust
/// Implement `Default` by delegating to `Self::new()`.
///
/// Only valid for types whose `new()` takes no arguments.
///
/// # Example
/// ```rust
/// default_new!(CgroupV2Limiter, OverlayFilesystem);
/// ```
#[macro_export]
macro_rules! default_new {
    ($($t:ty),+ $(,)?) => {
        $(
            impl Default for $t {
                fn default() -> Self {
                    Self::new()
                }
            }
        )+
    };
}
````

### Combined convenience macro

For adapter files that always need both:

```rust
/// Implement both `AsAny` and `Default` for the same list of types.
#[macro_export]
macro_rules! adapt {
    ($($t:ty),+ $(,)?) => {
        $crate::as_any!($($t),+);
        $crate::default_new!($($t),+);
    };
}
```

Usage at the end of each adapter file:

```rust
adapt!(ColimaRegistry, ColimaFilesystem, ColimaLimiter, ColimaRuntime);
```

---

## Placement

- Macro definitions: `mbx/src/macros.rs` (new file), re-exported from `lib.rs` via `pub use macros::*`
- `#[macro_export]` makes them available crate-wide without explicit `use`

## Why `macro_rules!` (not proc macro)

Both patterns are structurally trivial — identical body, one type variable. `macro_rules!` handles
this cleanly. A proc macro would require a separate crate (`minibox-macros`), adding build
complexity with no benefit. The 2025 `macro_rules!` improvements goal (rust-lang/rust-project-goals)
confirms the direction of keeping simple patterns in declarative macros.

---

## Impact

| File                                                       | Instances removed | Lines saved |
| ---------------------------------------------------------- | ----------------- | ----------- |
| `mocks.rs`                                                 | 8                 | ~32         |
| `colima.rs`                                                | 8                 | ~32         |
| `gke.rs`                                                   | 6                 | ~24         |
| `wsl.rs`                                                   | 6                 | ~24         |
| `docker_desktop.rs`                                        | 6                 | ~24         |
| `limiter.rs`, `filesystem.rs`, `runtime.rs`, `registry.rs` | 8                 | ~32         |
| **Total**                                                  | **42**            | **~168**    |

Future adapters: zero boilerplate to write.

---

## Testing

Verify the macro generates valid code by compiling existing adapters after replacement.
The `AsAny` trait is exercised by every `downcast_ref` call in `handler_tests.rs` — those
tests serve as the regression suite.
