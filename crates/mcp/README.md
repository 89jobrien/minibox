# minibox-mcp

`minibox-mcp` publishes the `mcp` binary and Rust library crate. It exposes a
local MCP stdio server that lets MCP clients inspect and control a running
`miniboxd` daemon through the existing Unix socket protocol.

The first implementation slice is intentionally local and bounded:

- stdio MCP transport only
- existing `minibox-core` daemon protocol only
- read-only inspection tools by default
- controlled run/pull/stop/rm tools guarded by agent policy

Tracing is written to stderr so stdout remains reserved for MCP frames.
