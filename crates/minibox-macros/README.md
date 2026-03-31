# minibox-macros

Procedural macros for minibox.

## Macros

- `as_any!` — Downcast trait objects to concrete types (used by mbx adapters for hexagonal architecture)
- `adapt!` — Helper for adapter registry pattern

These macros resolve to types in the calling crate, not the macro definition crate. Used internally by mbx.
