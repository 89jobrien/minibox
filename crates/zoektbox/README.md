# zoektbox

Zoekt code-search service adapter for minibox.

Manages the lifecycle of [Zoekt](https://github.com/sourcegraph/zoekt) — a fast trigram-based
code search engine — on a remote VPS. Handles binary distribution, deployment, and health
monitoring.

## Features

- Platform detection (Linux amd64/arm64, macOS arm64)
- Pinned release manifest with per-platform SHA256 checksums
- VPS provisioning via `go install` (Zoekt does not publish pre-built binaries)
- Remote deployment and service adapter

## Modules

| Module    | Description                                                              |
| --------- | ------------------------------------------------------------------------ |
| `release` | Pinned version manifest, platform detection, and binary names            |
| `download`| Tarball download with SHA256 verification                                |
| `deploy`  | Remote deployment via SSH / cargo xtask                                  |
| `service` | `ZoektServiceAdapter` — start, stop, health-check the remote service     |

## Pinned release

```rust
use zoektbox::ZOEKT_VERSION;
println!("deploying zoekt {ZOEKT_VERSION}");
```

The version, platform URLs, and SHA256 digests are pinned in `release.rs`. Update them when
upgrading:

1. Bump `ZOEKT_VERSION`
2. Download each platform tarball and run `sha256sum`
3. Update `expected_sha256` with the new digests

## Provisioning

Zoekt is installed via `go install` on the target VPS because it does not publish GitHub
release binaries. `ZoektServiceAdapter::provision()` runs:

```
GOBIN=/opt/zoekt/bin go install github.com/sourcegraph/zoekt/cmd/...@latest
```

Go must be present on the VPS `PATH`.

## Platform support

| Platform       | Support |
| -------------- | ------- |
| Linux amd64    | Yes     |
| Linux arm64    | Yes     |
| macOS arm64    | Yes     |
| Windows        | No      |
