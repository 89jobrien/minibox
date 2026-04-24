# minibox-testers

Test infrastructure for the minibox workspace — mocks, fixtures, conformance helpers, and
reporting utilities.

This crate is a `[dev-dependency]` only. It must never appear in production binaries. All
modules are `pub` so downstream test files can import them directly.

## Modules

| Module       | Description                                                              |
| ------------ | ------------------------------------------------------------------------ |
| `backend`    | `BackendDescriptor` and `BackendCapabilitySet` for conformance suites    |
| `capability` | `ConformanceCapability` trait and built-in capability descriptors         |
| `fixtures`   | Reusable test fixtures (temp image dirs, pre-baked layer data, etc.)     |
| `helpers`    | Common async helpers, polling utilities, and assertion extensions         |
| `mocks`      | Mock adapters for all domain traits (`MockRegistry`, `MockRuntime`, …)   |
| `report`     | Conformance report types — Markdown + JSON output                        |

## Conformance Capabilities

The `capability` module defines typed capability descriptors used by conformance suites to
decide whether to run or skip a test:

```rust
use minibox_testers::capability::{CommitCapability, should_skip};

let cap = CommitCapability { supported: false };
if let Some(reason) = should_skip(&cap) {
    eprintln!("skipping: {reason}");
    return;
}
```

Built-in capabilities: `Commit`, `BuildFromContext`, `PushToRegistry`, `ImageGarbageCollection`.

## Mock Adapters

`mocks::registry::MockRegistry` implements `ImageRegistry` for unit tests without network
calls. Configure at construction time:

```rust
use minibox_testers::mocks::registry::MockRegistry;

let registry = MockRegistry::with_pull_failure("simulated registry error");
let result = registry.pull_image(&image_ref).await;
assert!(result.is_err());
```

## Adding to a Crate

```toml
[dev-dependencies]
minibox-testers = { path = "../minibox-testers" }
```
