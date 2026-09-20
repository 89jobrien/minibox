# minibox-mcp

The Cargo package `minibox-mcp` publishes the `mcp` binary and the Rust library crate named `mcp`.
It is a local Model Context Protocol server that exposes typed minibox tools over stdio and calls a
running `miniboxd` through the existing Unix socket protocol.

The server does not implement a second runtime or bypass daemon policy:

```text
MCP client <-> mcp (stdio) <-> miniboxd (Unix socket) <-> selected adapter suite
```

## Running the server

```bash
cargo build -p minibox-mcp --release
RUST_LOG=mcp=debug ./target/release/mcp
```

The startup banner and tracing go to stderr. Stdout is reserved for MCP frames. Socket resolution
uses `MINIBOX_SOCKET_PATH`, `MINIBOX_RUN_DIR`, and then the platform default inherited from
`minibox-core`.

Example client configuration:

```json
{
  "command": "/absolute/path/to/target/release/mcp",
  "env": {
    "MINIBOX_SOCKET_PATH": "/run/minibox/miniboxd.sock"
  }
}
```

## Tools

| Tool               | Purpose                                               | Default policy                 |
| ------------------ | ----------------------------------------------------- | ------------------------------ |
| `minibox_doctor`   | Check daemon socket connectivity                      | Allowed                        |
| `minibox_ps`       | List known containers                                 | Allowed                        |
| `minibox_images`   | List cached images                                    | Allowed                        |
| `minibox_logs`     | Fetch stored container logs                           | Allowed                        |
| `minibox_manifest` | Fetch an execution manifest                           | Allowed                        |
| `minibox_run`      | Run an ephemeral container and collect bounded output | Allowed safely                 |
| `minibox_pull`     | Pull an OCI image                                     | Denied without mutation opt-in |
| `minibox_stop`     | Stop a container by ID or name                        | Denied without mutation opt-in |
| `minibox_rm`       | Remove a stopped container by ID or name              | Denied without mutation opt-in |

`minibox_run` defaults to an ephemeral, auto-removed, unprivileged container with no network.
Default limits are 512 MiB memory and CPU weight 100 when the caller omits them.

## Agent policy

| Variable                         | Enables                                              | Default |
| -------------------------------- | ---------------------------------------------------- | ------- |
| `MINIBOX_MCP_ALLOW_MUTATION`     | Pull, stop, remove, and authorized generic mutations | false   |
| `MINIBOX_MCP_ALLOW_BIND_MOUNTS`  | Bind mounts in `minibox_run`                         | false   |
| `MINIBOX_MCP_ALLOW_PRIVILEGED`   | Privileged `minibox_run`                             | false   |
| `MINIBOX_MCP_ALLOW_HOST_NETWORK` | Host network mode in `minibox_run`                   | false   |
| `MINIBOX_MCP_MAX_OUTPUT_BYTES`   | Maximum collected container output                   | 1 MiB   |

Boolean opt-ins accept `1`, `true`, `yes`, or `on` in the explicitly supported case variants.
Unknown network modes and unclean mount paths are rejected before daemon connection.

The MCP policy is an additional boundary. The daemon's own bind-mount, privileged, peer-credential,
and adapter capability checks still apply.

## Output and errors

Run output is decoded from daemon base64 frames into separate stdout and stderr strings. Container
output is truncated at the configured limit and reports `truncated: true`; non-output metadata has
a separate 64 KiB cap. Other calls reject responses that exceed the configured serialized limit.

Tool failures are structured MCP errors with stable `minibox::mcp::*` diagnostic codes and a
`retryable` hint. Daemon errors, connection errors, invalid input, policy denials, protocol errors,
and output-limit errors remain distinguishable.

## Rust API

The main public types are `MiniboxMcpServer`, `MiniboxDaemonClient`, `AgentPolicy`,
`Authorized<T>`, `DaemonCallResult`, and the schema-facing input/output types in `mcp::types`.

Read-only generic calls need no authorization proof:

```rust,no_run
use mcp::client::MiniboxDaemonClient;
use minibox_core::protocol::DaemonRequest;

# async fn list() -> mcp::error::Result<()> {
let client = MiniboxDaemonClient::from_env();
let result = client.call(DaemonRequest::List).await?;
assert_eq!(result.terminal_type.as_deref(), Some("ContainerList"));
# Ok(())
# }
```

Mutating generic calls require an unforgeable proof returned by the active policy:

```rust,no_run
use mcp::{client::MiniboxDaemonClient, policy::AgentPolicy};
use minibox_core::protocol::DaemonRequest;

# async fn stop(container_id: String) -> mcp::error::Result<()> {
let client = MiniboxDaemonClient::from_env();
let policy = AgentPolicy::from_env();
let request = policy.authorize_mutation(
    "custom_stop",
    DaemonRequest::Stop { id: container_id },
)?;
let (_result, _truncated) = client
    .call_authorized(request, policy.max_output_bytes)
    .await?;
# Ok(())
# }
```

## Constraints

- Stdio is the only MCP transport.
- The daemon transport is currently Unix-socket based; Windows daemon transport is not ready.
- There are no snapshot, pipeline, exec, event-stream, build, push, or verify tools in this first
  tool set, although some corresponding read-only requests can be made through the Rust client.
- The package is publishable, but operational use still requires a compatible local `miniboxd`.

## Development and testing

Unit tests cover policy, schema conversion, response accounting, tool mapping, and authorization.
Integration tests spawn a real MCP client/server pair with a mock daemon and verify tool discovery,
request translation, policy denial, error kinds, and bounded output.

```bash
cargo check -p minibox-mcp
cargo clippy -p minibox-mcp --all-targets -- -D warnings
cargo nextest run -p minibox-mcp
cargo xtask verify
```
