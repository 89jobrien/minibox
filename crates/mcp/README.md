# minibox-mcp

`minibox-mcp` publishes the `mcp` binary and Rust library crate. It exposes a
local MCP stdio server that lets MCP clients inspect and control a running
`miniboxd` daemon through the existing Unix socket protocol.

The first implementation slice is intentionally local and bounded:

- stdio MCP transport only
- existing `minibox-core` daemon protocol only
- read-only inspection tools by default
- controlled run/pull/stop/rm tools guarded by agent policy

## Permission model

Everything is deny-by-default except the core agent workflow:

| Tool                                                                                 | Default | Opt-in                                |
| ------------------------------------------------------------------------------------ | ------- | ------------------------------------- |
| `minibox_doctor`, `minibox_ps`, `minibox_images`, `minibox_logs`, `minibox_manifest` | allowed | —                                     |
| `minibox_run` (ephemeral, unprivileged, no network)                                  | allowed | —                                     |
| `minibox_run` with `privileged`                                                      | denied  | `MINIBOX_MCP_ALLOW_PRIVILEGED=true`   |
| `minibox_run` with bind mounts                                                       | denied  | `MINIBOX_MCP_ALLOW_BIND_MOUNTS=true`  |
| `minibox_run` with `network: host`                                                   | denied  | `MINIBOX_MCP_ALLOW_HOST_NETWORK=true` |
| `minibox_pull`, `minibox_stop`, `minibox_rm`                                         | denied  | `MINIBOX_MCP_ALLOW_MUTATION=true`     |

The asymmetry is deliberate: an ephemeral, isolated, auto-removed run is the
tool an agent exists to call, so it stays available without configuration,
while anything that escalates privileges or mutates shared daemon state
(pulled image cache, other containers' lifecycle) requires an explicit
operator opt-in.

`MINIBOX_MCP_MAX_OUTPUT_BYTES` bounds collected daemon output (default 1 MiB);
malformed values are logged and ignored.

Tracing is written to stderr so stdout remains reserved for MCP frames. Tool
failures are returned as structured MCP errors carrying a stable
`minibox::mcp::*` diagnostic code and a `retryable` hint in the error data.
