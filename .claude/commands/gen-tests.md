---
name: gen-tests
description: >
  Scaffold unit tests for a new minibox domain trait adapter using Claude.
  Use when implementing a new adapter and want a test skeleton generated.
argument-hint: "<TraitName> [--output <path>]"
agent: atelier:forge
allowed-tools: [Bash, Read, Write]
---

Scaffold a test module for a domain trait adapter.

Positional arg: trait name (e.g. `BridgeNetworking`). Optional: `--output <path>`.

Parse `$ARGUMENTS` for the trait name (first non-flag token) and `--output <path>`.

**Steps:**

1. Read `minibox-core/src/domain.rs` — locate the trait definition for `<TraitName>`
2. Find the adapter implementation under `crates/minibox/src/adapters/` that implements
   the trait (search for `impl <TraitName> for`)
3. Read `crates/minibox/src/adapters/mocks.rs` (or equivalent) for mock patterns
4. Scaffold a test module following the existing adapter test pattern:
   - `#[cfg(test)]` module at the bottom of the adapter file, or a separate `tests.rs`
   - One test per trait method
   - Linux-only tests gated with `#[cfg(target_os = "linux")]`
   - Use mock adapters from `adapters::mocks`
   - Use `expect("reason")` not `.unwrap()` in tests
5. If `--output <path>` provided: write to that path
   Otherwise: determine from adapter module location and write there

Print the path where the test module was written.
