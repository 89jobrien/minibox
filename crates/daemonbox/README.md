# daemonbox

Unix socket server, request handler, and container state management extracted from `miniboxd`. Platform-agnostic — used by Linux, macOS, and Windows daemon implementations.

## Contents

- **server.rs** — Unix socket listener with `SO_PEERCRED` authentication (UID==0 check)
- **handler.rs** — Request routing and processing (run, list, stop, remove, logs)
- **state.rs** — In-memory container tracking with optional persistence

## Handler Patterns

- `handle_run_streaming()` — Ephemeral containers on Linux; streams stdout/stderr back to client
- `handle_run()` — Daemon-managed containers; returns container ID immediately
- `handle_list()`, `handle_stop()`, `handle_remove()` — Standard container lifecycle

## Socket Communication

Messages flow as JSON-over-newline. Each message is tagged with a request ID for response correlation. Client connection inherits container output channels via tokio broadcast.

## State Lifecycle

Container records start in `Created`, transition to `Running`, then `Stopped`. The daemon persists state to disk (when enabled) after each transition.
