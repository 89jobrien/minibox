---
name: gen-tests
description: >
  Scaffold unit tests for a new minibox domain trait adapter using Claude.
  Use when implementing a new adapter and want a test skeleton generated.
argument-hint: "<TraitName> [--output <path>]"
---

# gen-tests

Scaffolds unit tests for a domain trait adapter. Pass the trait name as the
positional argument.

```nu
nu scripts/gen-tests.nu BridgeNetworking
nu scripts/gen-tests.nu ContainerRuntime --output crates/minibox/src/adapters/tests.rs
```

Claude reads the trait definition from `minibox-core/src/domain.rs` and generates
a test skeleton in the appropriate adapter module.
