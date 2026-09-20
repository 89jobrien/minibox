# minibox-macros

`minibox-macros` contains small declarative `macro_rules!` helpers shared across the minibox
workspace. It is a normal library crate, not a procedural-macro crate.

## Public macros

| Macro                 | Purpose                                                          |
| --------------------- | ---------------------------------------------------------------- |
| `as_any!`             | Implement the call site's `domain::AsAny` trait                  |
| `default_new!`        | Implement `Default` through a zero-argument `new()`              |
| `adapt!`              | Apply both `as_any!` and `default_new!`                          |
| `normalize_name!`     | Replace `/` with `_` for a path component                        |
| `normalize_digest!`   | Replace `:` with `_` for a path component                        |
| `normalize!`          | Replace both `/` and `:` with `_`                                |
| `denormalize_digest!` | Replace `_` with `:` in a normalized digest                      |
| `require_capability!` | Return early from a test when a host capability is absent        |
| `test_run!`           | Build a `DaemonRequest::Run` with test defaults and overrides    |
| `unsafe_set_var!`     | Set an environment variable in serialized Rust 2024 tests        |
| `unsafe_remove_var!`  | Remove an environment variable in serialized Rust 2024 tests     |
| `unsafe_restore_var!` | Restore a previously captured environment value in tests         |
| `provide!`            | Generate environment constructors for a compatible provider type |

## Examples

Normalization macros are self-contained:

```rust
use minibox_macros::{normalize, normalize_digest, normalize_name};

assert_eq!(normalize_name!("library/alpine"), "library_alpine");
assert_eq!(normalize_digest!("sha256:abc"), "sha256_abc");
assert_eq!(normalize!("ghcr.io/acme/app:v1"), "ghcr.io_acme_app_v1");
```

Adapter helpers expect a compatible call-site API:

```rust,ignore
use minibox_macros::adapt;

// The calling crate must expose `crate::domain::AsAny`, and each type must
// provide `fn new() -> Self`.
adapt!(MyRuntime, MyFilesystem);
```

`test_run!` expands against `minibox_core::protocol::TestRunDefaults`:

```rust,ignore
use minibox_macros::test_run;

let request = test_run!(
    image: "alpine".to_string(),
    command: vec!["echo".to_string(), "hello".to_string()],
);
```

## Call-site constraints

- `as_any!` intentionally uses `crate::domain::AsAny`; `crate` resolves at the invocation site.
- `adapt!` and `default_new!` require each type to expose a zero-argument `new()` method.
- `provide!` expects `crate::ProviderConfig` and provider constructors with matching signatures.
- Environment macros are test helpers, not synchronization. The caller must hold a process-wide
  mutex while mutating environment variables.
- `denormalize_digest!` is only safe when underscores are known to represent replaced colons; it
  is not a general inverse for arbitrary input.

## Development and testing

The conformance test exercises expansion from a downstream crate, which is important for macro
hygiene and call-site path resolution.

```bash
cargo check -p minibox-macros
cargo clippy -p minibox-macros --all-targets -- -D warnings
cargo nextest run -p minibox-macros
cargo test -p minibox-macros --doc
```
