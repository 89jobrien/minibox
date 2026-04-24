---
status: active
note: Integration analysis — updated 2026-04-24 to reflect current minibox capabilities
---

# Maestro-Minibox

## What Integration Actually Requires

### The `ContainerProvider` Trait Surface

Maestro's trait has these methods (simplified):

```rust
create_container(name, config) -> Result<String>   // create + start
exists(name) -> Result<bool>
is_running(name) -> Result<bool>
exec(name, cmd) -> Result<ExecOutput>               // stdout/stderr/exit_code
stop(name) -> Result<()>
cleanup(name_pattern) -> Result<()>
logs(name) -> Result<String>
list(pattern) -> Result<Vec<ContainerInfo>>
architecture() -> Result<String>
// tmux wrappers — all delegate to exec() by default
list_tmux_sessions, tmux_session_exists, capture_tmux_pane, tmux_send_keys
```

### Gap Analysis: Minibox vs. What Maestro Needs

> Updated 2026-04-24 — many items from the original gap table are now implemented.

| Feature                          | Minibox today                              | Remaining gap                                           |
| -------------------------------- | ------------------------------------------ | ------------------------------------------------------- |
| `exists` / `is_running` / `list` | `DaemonRequest::List` ✓                    | None — filter list by name via `resolve_id()`           |
| `stop` / `cleanup`               | `Stop` + `Remove` ✓                        | None — name lookup via `resolve_id()` implemented       |
| `create_container`               | `Run { name: Option<String> }` ✓           | None — named containers fully implemented               |
| `exec` (non-interactive)         | `DaemonRequest::Exec` ✓ (Linux native)     | macOS/non-native adapters return error (expected)       |
| `logs`                           | `DaemonRequest::ContainerLogs` ✓ (partial) | **File capture not wired** — stdio still goes to daemon |
| `architecture`                   | Not in protocol                            | Easy add — 1 new request variant + `uname` call         |
| **TTY/stdio piping**             | Designed, not runtime-implemented          | Core blocker for interactive `execute_local_direct`     |
| **Networking**                   | Bridge adapter exists, not wired into init | Hard blocker for browser terminal; deferred for CLI     |

### What Changed Since the Original Analysis

**Implemented (no longer blockers):**

- `DaemonRequest::Exec { container_id, cmd, env, working_dir, tty }` — full streaming exec via
  `setns(2)` on Linux native. Returns `ExecStarted` then streams `ContainerOutput` +
  `ContainerStopped`. `PtySessionRegistry` handles `SendInput` and `ResizePty` for interactive
  sessions.
- Named containers — `DaemonRequest::Run` has `name: Option<String>`; `DaemonState::resolve_id()`
  resolves by ID or name; `name_in_use()` prevents duplicates. All handlers accept `name_or_id`.
- Stdout/stderr streaming — `ContainerOutput { stream, data }` (base64) is the protocol for
  ephemeral containers; pipe-based capture is wired in `process.rs`.
- `ContainerLogs { container_id, follow }` protocol variant exists with `LogLine` streaming
  responses. `follow` is parsed but not yet acted on.
- `SendInput` / `ResizePty` — stdin forwarding and PTY resize fully wired for exec sessions.
- Pause/resume — `PauseContainer` / `ResumeContainer` via cgroup freeze.

**Designed but not runtime-implemented:**

- PTY/TTY — `ContainerConfig::pty`, `PtyAllocator` trait, `tty: bool` on Run and Exec all exist.
  No `posix_openpt()` call in `process.rs`; no PTY master fd returned to the client socket.
- Bridge networking — `BridgeNetwork`, `IpAllocator`, iptables DNAT all implemented. Not called
  from `handler.rs`; container processes get an isolated net namespace but no veth attached.

---

### The Remaining Blockers

#### 1. Log file capture (Phase 1 — small, self-contained)

The `ContainerLogs` protocol and `handle_logs()` handler exist and stream `LogLine` responses
from `{containers_base}/{id}/stdout.log` and `{id}/stderr.log`. However, `process.rs` still
sends container stdio to the daemon's inherited fds — those files are never written.

**What it takes:** In `child_init`, open `{data_dir}/containers/{id}/stdout.log` and
`{id}/stderr.log` for writing before exec, then `dup2()` them onto stdout/stderr fds. Small,
self-contained change in `process.rs`.

#### 2. `architecture()` method (trivial)

Not in the protocol. Add `DaemonRequest::Architecture` → `DaemonResponse::Architecture {
arch: String }`, implement by running `uname -m` in the daemon process.

#### 3. TTY / stdio piping (Phase 2 — interactive attach)

The trait infrastructure (`PtyAllocator`, `PtyConfig`, `PtySessionRegistry`) is designed and
partially present. The runtime gap is the actual PTY allocation in `process.rs`:

1. When `ContainerConfig::pty` is set, call `posix_openpt()` → master + slave fds
2. Pass slave fd to child as stdin/stdout/stderr (cannot be `O_CLOEXEC`)
3. Parent reads master fd and writes `ContainerOutput` to the socket
4. Reads from socket (`SendInput`) written to master fd

The existing newline-JSON framing handles binary via base64 encoding — the `ContainerOutput` /
`SendInput` protocol is already sufficient for PTY forwarding. No new framing mode needed.

#### 4. Networking (Phase 3 — browser terminal only)

`BridgeNetwork` (veth pairs, iptables DNAT, IP allocation) is implemented in
`crates/minibox/src/adapters/network/bridge.rs`. The integration gap is wiring it into
`handler.rs`: call `network_provider.setup()` before container spawn, attach the veth
container-end inside the child's net namespace. Required only for the maestro-ui browser
terminal path; CLI-only tmux sessions do not need it.

---

### Revised MVP Integration Plan

**Phase 1 — CLI-only sessions (unblocked, ~1 day of work):**

1. ~~Add `ContainerName` to `DaemonRequest::Run`~~ — **done**
2. ~~Add `DaemonRequest::Exec`~~ — **done**
3. Wire log file capture: in `process.rs`, redirect child stdio → `{id}/stdout.log` /
   `{id}/stderr.log` before exec
4. Add `DaemonRequest::Architecture` → `uname -m` response
5. Implement `ContainerProvider` in maestro-cli backed by minibox's Unix socket

This gets you `maestro start --runtime minibox` with CLI-only tmux attachment.

**Phase 2 — Interactive TTY attach (~2–3 days):**

6. PTY allocation in `process.rs` when `ContainerConfig::pty = true`
7. Wire `PtyAllocator` impl into the native adapter suite in `miniboxd`
8. Hook into maestro's `execute_local_direct` attach path

**Phase 3 — Browser terminal (deferred, ~1 week):**

9. Wire `BridgeNetwork` into `handler.rs` container creation path
10. Configure container-end veth and IP inside child net namespace
11. Container gets a real IP; maestro proxy routing works

---

### Bottom Line

Phase 1 is now **two small code changes** (log file capture in `process.rs` + `Architecture`
protocol variant) plus the `ContainerProvider` impl in maestro. The original "core blockers"
(`exec`, named containers, streaming) are resolved. TTY and networking remain Phase 2/3 work.
