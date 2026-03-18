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

| Feature                          | Minibox today           | Gap                                                |
| -------------------------------- | ----------------------- | -------------------------------------------------- |
| `exists` / `is_running` / `list` | `DaemonRequest::List` ✓ | Trivial — filter list by name                      |
| `stop` / `cleanup`               | `Stop` + `Remove` ✓     | Named container lookup needed                      |
| `create_container`               | `Run` ✓ (partially)     | No named containers, no `ContainerName` field      |
| `architecture`                   | Not in protocol         | Easy — read `/proc/cpuinfo` or `uname`             |
| `logs`                           | **Missing**             | Stdout/stderr are discarded post-`execvp`          |
| **`exec`**                       | **Missing**             | Core blocker — needs `setns(2)` + output streaming |
| **TTY/stdio**                    | **Missing**             | Core blocker — needs PTY allocation                |
| **Networking**                   | **Missing**             | Hard blocker for proxy-based terminal              |

---

### The Two Real Blockers

#### 1. `exec` into a running container

Maestro does everything through exec: tmux session management, provisioning checks, file uploads, etc.

What it takes in minibox:

1. New protocol message: `DaemonRequest::Exec { id, cmd, args }`
2. Daemon reads `/proc/{pid}/ns/{pid,mnt,net,...}` from the tracked container PID
3. Calls `setns(2)` to enter each namespace
4. Forks child in those namespaces, captures stdout/stderr via pipes
5. Streams output back to CLI — **but the current protocol is one-response-per-request**, so this requires multiplexed streaming or accumulating output until completion

For non-interactive exec (what maestro mostly needs — running `tmux ls`, checking state) the simpler approach works: accumulate output and return one `ExecOutput` response. No streaming needed initially.

#### 2. TTY / stdio piping

Currently in `process.rs`, `child_init` inherits the daemon's fds — meaning container stdout goes to the daemon's terminal, not back to the client. Maestro needs to attach to container stdio for `create_container` (the session init logs) and for direct terminal attach (`execute_local_direct`).

What it takes:

1. Before forking, allocate a PTY pair: `posix_openpt()` → master + slave fds
2. Pass PTY slave to child (cannot be `O_CLOEXEC`) as its stdin/stdout/stderr
3. New protocol mode: after `ContainerCreated`, socket switches to raw PTY-forwarding stream
4. Daemon reads master fd → writes to socket; reads from socket → writes to master fd

This is the same architecture Docker uses for `attach`. The protocol needs to grow a streaming mode — current newline-JSON framing doesn't support interleaved binary data.

#### 3. Networking (hardest, most deferrable)

Maestro's proxy server routes WebSocket terminal connections to the container's terminal server via container IP. Without bridge networking in minibox, there's **no IP to route to**.

However — this path is **only needed for the browser-based terminal** (maestro-ui). The CLI path (`execute_with_docker` → `docker exec` → tmux) doesn't use networking at all. So if you constrain integration to CLI-only sessions, networking can be deferred indefinitely.

---

### Realistic MVP Integration Plan

**Phase 1 — CLI-only sessions (no networking required):**

1. Add `ContainerName` to minibox's `DaemonRequest::Run` + state tracking by name
2. Add `DaemonRequest::Exec { id, cmd, args }` — non-streaming, returns full output
3. Add log capture: redirect container stdout/stderr to a file, serve via `DaemonRequest::Logs { id }`
4. Implement `ContainerProvider` in maestro-cli backed by minibox's Unix socket

This gets you `maestro start --runtime minibox` with CLI-only tmux attachment.

**Phase 2 — TTY attach:**

5. PTY allocation in minibox daemon
6. New streaming protocol mode (binary framing post-handshake)
7. Hook into maestro's `execute_local_direct` attach path

**Phase 3 — Browser terminal (if ever needed):**

8. veth/bridge networking in minibox
9. Container gets a real IP, proxy routing works

---

### Bottom Line

The simplest path to something working is Phase 1: ~3 protocol changes in minibox + a new `ContainerProvider` impl in maestro. No TTY, no networking — but all the provisioning, tmux session management, and CLI attach would work.
