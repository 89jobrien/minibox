# minibox-core

Cross-platform protocol, client, image, and shared adapter infrastructure for minibox.

## Contents

- **Protocol** — JSON-over-newline messages for daemon/CLI communication (run, list, stop, remove containers)
- **Domain facade** — compatibility re-exports of ports canonically owned by `minibox-domain`
- **Image management** — `ImageStore`, `RegistryClient` abstractions; layer caching and manifest parsing
- **Error types** — Unified `ImageError`, `ContainerError` enums across platforms
- **Preflight** — Host capability probing (cgroups v2, overlay, namespaces, kernel version)

## Re-exports

`minibox` re-exports all public types from `minibox-core` for convenience. Prefer direct imports in new code outside minibox.

## Test Utilities

Enable the `test-utils` feature to access mock adapters (`MockRegistry`, `MockFilesystem`, etc.) for use in integration tests without cfg(test) restrictions.
