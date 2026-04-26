---
status: open
---

# Dogfood-1 MCP Server — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an MCP server crate (`minibox-mcp`) that exposes minibox daemon commands as
Claude-compatible tools (`pull_image`, `run_container`, `ps`, `stop`, `rm`, `exec`). Claude
can then orchestrate containers in a real agent loop over the Unix socket protocol, exercising
streaming output and error reporting end-to-end.

**Architecture:** Thin MCP transport layer over the existing `minibox_core::client::DaemonClient`.
No new daemon features required. The MCP server is a standalone binary that connects to
`miniboxd` as a regular client. SOLID: the MCP crate depends only on `minibox-core` (protocol
types + client); it never imports `minibox` (linuxbox) or `miniboxd`.

**Tech Stack:** Rust, `rmcp` (Rust MCP SDK), `minibox-core` (client + protocol), `tokio`,
`serde_json`, `cargo nextest`.

---

## Causal Chain

```text
T1: Scaffold minibox-mcp crate              (prereq for all)
  └─► T2: Implement tool handlers            (core logic)
        ├─► T3: Streaming run tool            (most complex tool)
        └─► T4: Unit tests with mock socket   (correctness)
              └─► T5: Integration smoke test  (e2e proof)
                    └─► T6: Claude config + docs
                          └─► T7: Commit + PR
```

**Note:** T3 and T4 can proceed in parallel once T2 is done. T5 requires a running daemon
(Linux or macOS with adapter).

---

## File Map

| Action    | Path                                    |
| --------- | --------------------------------------- |
| Create    | `crates/minibox-mcp/Cargo.toml`         |
| Create    | `crates/minibox-mcp/src/main.rs`        |
| Create    | `crates/minibox-mcp/src/tools.rs`       |
| Create    | `crates/minibox-mcp/src/streaming.rs`   |
| Modify    | `Cargo.toml` (workspace members)        |
| Create    | `crates/minibox-mcp/tests/mock_tools.rs`|
| Reference | `crates/minibox-core/src/client/socket.rs` |
| Reference | `crates/minibox-core/src/protocol.rs`   |

---

## Task 1: Scaffold minibox-mcp crate

**Files:** `Cargo.toml` (workspace), `crates/minibox-mcp/Cargo.toml`,
`crates/minibox-mcp/src/main.rs`

- [ ] **Step 1: Create crate directory and Cargo.toml**

  ```toml
  [package]
  name = "minibox-mcp"
  version.workspace = true
  edition.workspace = true
  license.workspace = true
  description = "MCP server exposing minibox container commands as Claude tools"

  [dependencies]
  minibox-core = { path = "../minibox-core" }
  rmcp = { version = "0.1", features = ["server", "transport-stdio"] }
  tokio = { version = "1", features = ["full"] }
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  anyhow = "1"
  tracing = "0.1"
  tracing-subscriber = { version = "0.3", features = ["env-filter"] }
  ```

- [ ] **Step 2: Add to workspace members**

  Add `"crates/minibox-mcp"` to the `[workspace] members` array in root `Cargo.toml`.

- [ ] **Step 3: Stub main.rs with MCP server skeleton**

  ```rust
  use anyhow::Result;

  mod tools;

  #[tokio::main]
  async fn main() -> Result<()> {
      tracing_subscriber::fmt()
          .with_env_filter("minibox_mcp=info")
          .init();
      tracing::info!("minibox-mcp: starting MCP server on stdio");
      tools::serve_stdio().await
  }
  ```

- [ ] **Step 4: Verify it compiles**

  ```bash
  cargo check -p minibox-mcp
  ```

---

## Task 2: Implement tool handlers

**Files:**

- Create: `crates/minibox-mcp/src/tools.rs`

**Implementation:** Each MCP tool maps 1:1 to a `DaemonRequest` variant. The handler
creates a `DaemonClient`, sends the request, reads the response, and returns a JSON
tool result.

- [ ] **Step 1: Define tool schemas**

  Tools to expose:
  - `pull_image` — params: `image: string`, `tag?: string`
  - `run_container` — params: `image: string`, `tag?: string`, `command: string[]`,
    `memory_limit_mb?: number`, `env?: string[]`, `mounts?: string[]`, `name?: string`
  - `ps` — no params
  - `stop` — params: `id: string`
  - `rm` — params: `id: string`
  - `exec` — params: `container_id: string`, `cmd: string[]`, `env?: string[]`

- [ ] **Step 2: Implement non-streaming handlers (pull, ps, stop, rm)**

  Each handler: connect to socket via `DaemonClient::connect()`, send request, read
  single response, format as tool result text.

- [ ] **Step 3: Wire into rmcp Server and implement `serve_stdio()`**

  Register all tools with the MCP server, start stdio transport.

- [ ] **Step 4: Verify compilation**

  ```bash
  cargo check -p minibox-mcp
  ```

---

## Task 3: Streaming run tool

**Files:**

- Create: `crates/minibox-mcp/src/streaming.rs`

**Change:** `run_container` uses `ephemeral: true` and must consume the
`ContainerOutput` stream until `ContainerStopped`. Collect all output and return as a
single tool result (MCP tools are request-response, not streaming).

- [ ] **Step 1: Implement run handler with output collection**

  ```rust
  // Pseudocode:
  // 1. Send Run { ephemeral: true, ... }
  // 2. Loop: read DaemonResponse lines
  //    - ContainerCreated: record id
  //    - ContainerOutput: append to output buffer
  //    - ContainerStopped: break, return collected output + exit code
  //    - Error: return error
  ```

- [ ] **Step 2: Add timeout handling**

  Use `tokio::time::timeout` to cap container execution at 5 minutes (configurable via
  `MINIBOX_MCP_TIMEOUT_SECS` env var). Return partial output on timeout.

- [ ] **Step 3: Wire into tools.rs run_container handler**

---

## Task 4: Unit tests with mock socket

**Files:**

- Create: `crates/minibox-mcp/tests/mock_tools.rs`

- [ ] **Step 1: Write test for ps tool**

  Spawn a mock Unix socket that returns a `ContainerList` response. Call the ps handler.
  Assert the tool result contains the expected container info.

- [ ] **Step 2: Write test for run tool (streaming)**

  Mock socket returns `ContainerCreated` + 2x `ContainerOutput` + `ContainerStopped`.
  Assert collected output matches and exit code is correct.

- [ ] **Step 3: Write test for error handling**

  Mock socket returns `Error { message }`. Assert tool result is an error.

- [ ] **Step 4: Run tests**

  ```bash
  cargo nextest run -p minibox-mcp
  ```

---

## Task 5: Integration smoke test

**Files:** none (manual verification on Linux/macOS with running daemon)

- [ ] **Step 1: Build and run with echo test**

  ```bash
  cargo build -p minibox-mcp
  echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | \
    ./target/debug/minibox-mcp
  ```

- [ ] **Step 2: Verify tool invocation against live daemon**

  ```bash
  # With miniboxd running:
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ps"}}' | \
    ./target/debug/minibox-mcp
  ```

---

## Task 6: Claude config + docs

**Files:**

- Create: `docs/mcp-server.md`

- [ ] **Step 1: Write usage documentation**

  Document how to add minibox-mcp to Claude Code's MCP config:
  ```json
  {
    "mcpServers": {
      "minibox": {
        "command": "/path/to/minibox-mcp",
        "args": []
      }
    }
  }
  ```

- [ ] **Step 2: Document available tools and their parameters**

---

## Task 7: Commit + PR

- [ ] **Step 1: Run full workspace check**

  ```bash
  cargo xtask pre-commit
  cargo nextest run -p minibox-mcp
  ```

- [ ] **Step 2: Stage and commit**

  ```bash
  git add crates/minibox-mcp/ Cargo.toml docs/mcp-server.md
  git commit -m "feat(mcp): add minibox-mcp server crate

  Thin MCP server exposing minibox daemon commands as Claude-compatible
  tools (pull, run, ps, stop, rm, exec). Uses stdio transport and
  connects to miniboxd via Unix socket. Streaming run collects output
  and returns as single tool result.

  Ref: docs/ROADMAP.md dogfood item 1"
  ```

- [ ] **Step 3: Push and open PR**

  ```bash
  git push origin HEAD
  gh pr create \
    --title "feat(mcp): minibox-mcp server crate" \
    --body "$(cat <<'EOF'
  ## Summary
  - New `minibox-mcp` crate: MCP server for Claude to control minibox
  - Tools: pull_image, run_container, ps, stop, rm, exec
  - Streaming run with timeout and output collection
  - Unit tests with mock socket

  ## Test plan
  - [ ] cargo nextest run -p minibox-mcp
  - [ ] cargo xtask pre-commit
  - [ ] Manual smoke test with Claude Code MCP config
  EOF
  )"
  ```

---

## Self-Review

**Spec coverage check:**

| Gap / objective                      | Task |
| ------------------------------------ | ---- |
| MCP tool schemas for all commands    | T2   |
| Streaming run output collection      | T3   |
| Error handling for daemon errors     | T4   |
| Claude Code integration docs         | T6   |

**Placeholder scan:** All placeholders filled with concrete crate names, paths, and
tool names.

**Type consistency:** `DaemonClient`, `DaemonRequest`, `DaemonResponse`,
`ContainerOutput`, `ContainerStopped` — all from `minibox_core::client` and
`minibox_core::protocol`.
