# Daemon Protocol

`DaemonRequest` and `DaemonResponse` are the canonical tagged wire contract. `DaemonClient` sends newline-delimited JSON over a Unix socket; the daemon server authenticates, frames, dispatches, and streams responses. `mbx`, MCP, Crux, and TUI share this protocol.

See [[Minibox]] and [[Adapter Composition]].

Evidence: `crates/minibox-core/src/protocol.rs:104-873`, `crates/minibox-core/src/client/socket.rs:30-81`, `crates/minibox/src/daemon/server.rs:162-403`.
