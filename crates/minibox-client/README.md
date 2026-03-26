# minibox-client

Low-level async client for communicating with the minibox daemon over Unix socket.

## Usage

```rust
use minibox_client::DaemonClient;
use minibox_core::protocol::{Request, Command};

let client = DaemonClient::connect("/run/minibox/miniboxd.sock").await?;
let request = Request { id: 1, command: Command::Ps };
let response = client.send(request).await?;
```

## Message Protocol

JSON-over-newline on Unix socket. Each request/response includes:
- `id` — Correlation ID for multiplexing
- `type` — Message variant (e.g., "RunContainer", "ListContainers")
- Payload — Variant-specific fields

## Streaming

For ephemeral containers, the client receives a stream of `ContainerOutput` and `ContainerStopped` messages via broadcast channels, enabling real-time stdout/stderr display.
