# minibox-core

`minibox-core` owns the cross-platform infrastructure shared by minibox clients, adapters, and the
daemon. It sits between the pure `minibox-domain` contracts and the runtime-specific `minibox`
application layer.

## Architecture role

This crate is the canonical owner of the daemon wire protocol, Unix socket client, OCI image
services, shared adapters, tracing setup, and infrastructure error types. Its `domain` module is a
compatibility facade that re-exports the types canonically defined by `minibox-domain`.

New code outside the runtime crate should normally import from `minibox-core` or
`minibox-domain` directly. The `minibox` crate retains compatibility re-exports for existing
callers and macro expansion.

## Main modules

| Module                   | Purpose                                                                          |
| ------------------------ | -------------------------------------------------------------------------------- |
| `protocol`               | `DaemonRequest`, `DaemonResponse`, NDJSON framing, terminal-response rules       |
| `client`                 | `DaemonClient`, response streams, writers, and socket-path resolution            |
| `image`                  | Image store, manifests, secure layer extraction, registry client, GC, and leases |
| `adapters`               | Registry routing, Docker Hub adapter, no-op exec, mocks, and conformance helpers |
| `events`                 | Broadcast event broker and event-source adapter                                  |
| `domain`                 | Compatibility re-export of `minibox-domain`                                      |
| `path`                   | `ValidatedPath` and `InternalPath` compatibility export                          |
| `preflight`              | Host capability probing                                                          |
| `error`                  | `MiniboxError`, `ImageError`, `RegistryError`, and operation errors              |
| `progress`               | Tokio-backed progress sinks and exec-output stream adapters                      |
| `typestate`              | Compile-time container lifecycle states and valid transitions                    |
| `trace` / `tracing_init` | Trace context and default tracing initialization                                 |

## Protocol and client

Requests and responses are newline-delimited JSON. The default Unix socket path resolves in this
order:

1. `MINIBOX_SOCKET_PATH`;
2. `$MINIBOX_RUN_DIR/miniboxd.sock`; or
3. `/tmp/minibox/miniboxd.sock` on macOS and `/run/minibox/miniboxd.sock` elsewhere.

```rust,no_run
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

# async fn list() -> minibox_core::client::Result<()> {
let client = DaemonClient::new();
let mut responses = client.call(DaemonRequest::List).await?;
if let Some(DaemonResponse::ContainerList { containers }) = responses.next().await? {
    println!("{} containers", containers.len());
}
# Ok(())
# }
```

Streaming operations may return several non-terminal responses before a terminal response. Use
`DaemonResponse::is_terminal()` rather than assuming the first response completes a request.

## Image API

`ImageStore` persists manifests, image configuration, and extracted layers beneath a caller-owned
base directory. Layer extraction validates paths and special file types; verified storage hashes
compressed input before atomically publishing a layer directory.

```rust
use minibox_core::{ImageRef, ImageStore};

let image = ImageRef::parse("alpine:3.20")?;
let store = ImageStore::new(std::env::temp_dir().join("minibox-images"))?;
assert_eq!(image.tag, "3.20");
assert!(!store.has_image(&image.cache_name(), &image.tag));
# Ok::<(), anyhow::Error>(())
```

`RegistryClient` and `image::registry` are available only with the `registry` feature.

## Features

| Feature      | Default | Effect                                                                 |
| ------------ | ------- | ---------------------------------------------------------------------- |
| `registry`   | Yes     | Enable the reqwest-based OCI registry client                           |
| `test-utils` | No      | Export mocks, fixtures, conformance support, and domain test utilities |
| `fuzzing`    | No      | Expose internals required by the cargo-fuzz harnesses                  |

Disabling default features removes registry networking while preserving local image-store and
protocol functionality:

```toml
minibox-core = { version = "0.33", default-features = false }
```

## Platform constraints

- The data model, protocol, image store, and most shared adapters are cross-platform.
- The socket client uses Unix domain sockets; current daemon transport on Windows is not
  implemented.
- `libc` is included only on Unix.
- Host capability reports describe support; they do not grant privileges or configure the host.

## Development and testing

Integration tests cover protocol evolution and round trips, domain contracts, image references,
events, metrics, PTY behavior, error models, and slashcrux integration.

```bash
cargo check -p minibox-core --all-features
cargo clippy -p minibox-core --all-targets --all-features -- -D warnings
cargo nextest run -p minibox-core --all-features
cargo xtask test property
cargo xtask verify
```

Protocol changes begin in `src/protocol.rs` and must be reflected in daemon handlers, control
surfaces, protocol evolution tests, and protocol-drift baselines.
