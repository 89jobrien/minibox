---
status: archived
note: Never built; channel-based streaming shipped instead
---

# Macro: Streaming Response Protocol

**Date:** 2026-03-17
**Status:** Design
**Scope:** `mbx/src/protocol.rs`, `miniboxd/src/server.rs`, `minibox-cli/src/`

---

## Problem

The current protocol is strictly request-response: one JSON line in → one JSON line out.
This works for all existing commands. Maestro Phase 2 introduces two new interaction patterns
that this model cannot accommodate:

1. **`exec` with output capture** (Phase 1 simpler form): run a command in a container and
   return its stdout/stderr. For short-lived commands (tmux ls, file writes) the output
   fits in a single response. But for longer-running exec (provisioning scripts, test runs),
   the client needs progress without waiting for the process to finish.

2. **PTY attach** (Phase 2): bidirectional raw bytes after a handshake — container stdin,
   stdout, stderr multiplexed over the socket. Newline-JSON framing is incompatible with
   binary PTY data.

The macro design question: as the protocol grows to support streaming, how do we avoid
littering `server.rs` with ad-hoc state machine code for each new streaming command?

---

## Protocol Extension Design

Before designing the macro, the protocol itself needs to be specified.

### Framing: two modes

**Mode 1 (current): line-JSON** — one request, one response, newline-terminated.
**Mode 2 (new): framed streaming** — entered via a handshake; each frame has a 4-byte
length prefix + type byte + payload.

```
[u32 BE length][u8 frame_type][payload bytes...]
```

Frame types:

| Type | Name     | Direction       | Meaning                                  |
| ---- | -------- | --------------- | ---------------------------------------- |
| 0x01 | `Stdout` | daemon → client | stdout bytes from container/exec process |
| 0x02 | `Stderr` | daemon → client | stderr bytes                             |
| 0x03 | `Stdin`  | client → daemon | stdin bytes to forward                   |
| 0x04 | `Resize` | client → daemon | PTY resize (`[u16 rows][u16 cols]`)      |
| 0x05 | `Exit`   | daemon → client | process exited (`[i32 exit_code]`)       |
| 0x06 | `Error`  | daemon → client | error before exit                        |

Mode switch: the daemon sends a special JSON response `{"type":"StreamBegin","stream_id":"..."}`,
after which both sides switch to framed mode for that stream.

For non-interactive exec (Phase 1), only `Stdout`, `Stderr`, and `Exit` are used.
The client accumulates and returns when it receives `Exit`.

---

## Macro Design

### Goal

The macro handles the repetitive scaffolding of a streaming handler:

- open the streaming channel
- select! loop over: process output, client input, timeout, shutdown signal
- send frames to client
- clean up on exit

### `stream!`

````rust
/// Generate a streaming command handler.
///
/// The macro wires together:
/// - An `OwnedWriteHalf` to send frames to the client
/// - A `tokio::process::Child` or output-producing future
/// - A select! loop dispatching stdout/stderr/exit/error to the client
///
/// # Example
/// ```rust,ignore
/// stream! {
///     name: handle_exec_stream,
///     setup: |id, command, args, state, deps| {
///         // returns Result<(Child, ContainerPid)>
///         exec_setup(id, command, args, state, deps).await?
///     },
///     on_stdout: |bytes| Frame::Stdout(bytes),
///     on_stderr: |bytes| Frame::Stderr(bytes),
///     on_exit:   |code| Frame::Exit(code),
///     timeout:   Duration::from_secs(30),
/// }
/// ```
macro_rules! stream {
    (
        name: $fn_name:ident,
        setup: |$($setup_arg:ident),*| $setup_body:expr,
        timeout: $timeout:expr $(,)?
    ) => {
        pub async fn $fn_name(
            $($setup_arg: _,)*
            mut writer: tokio::net::unix::OwnedWriteHalf,
            state: Arc<DaemonState>,
            deps: Arc<HandlerDependencies>,
        ) -> anyhow::Result<()> {
            let mut child = $setup_body;

            let stdout = child.stdout.take().expect("stdout not captured");
            let stderr = child.stderr.take().expect("stderr not captured");

            let mut stdout_reader = tokio::io::BufReader::new(stdout);
            let mut stderr_reader = tokio::io::BufReader::new(stderr);
            let mut buf = vec![0u8; 4096];

            let deadline = tokio::time::Instant::now() + $timeout;
            loop {
                tokio::select! {
                    n = stdout_reader.read(&mut buf) => {
                        match n? {
                            0 => break,
                            n => send_frame(&mut writer, FrameType::Stdout, &buf[..n]).await?,
                        }
                    }
                    n = stderr_reader.read(&mut buf) => {
                        match n? {
                            0 => {}
                            n => send_frame(&mut writer, FrameType::Stderr, &buf[..n]).await?,
                        }
                    }
                    status = child.wait() => {
                        let code = status?.code().unwrap_or(-1);
                        send_frame(&mut writer, FrameType::Exit, &(code as i32).to_be_bytes()).await?;
                        break;
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        send_frame(&mut writer, FrameType::Error, b"timeout").await?;
                        let _ = child.kill().await;
                        break;
                    }
                }
            }
            Ok(())
        }
    };
}
````

The macro's primary value is enforcing that every streaming handler:

1. Has a timeout (forgetting it is a resource-leak bug)
2. Sends a terminal `Exit` or `Error` frame (the client blocks until one arrives)
3. Uses the same frame encoding function (`send_frame`)

Without the macro, each streaming handler author must remember all three invariants.

### `send_frame` helper (not a macro)

```rust
async fn send_frame(
    writer: &mut OwnedWriteHalf,
    frame_type: FrameType,
    payload: &[u8],
) -> anyhow::Result<()> {
    let len = (1 + payload.len()) as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&[frame_type as u8]).await?;
    writer.write_all(payload).await?;
    Ok(())
}
```

---

## Server Integration

`server.rs` needs to detect the `StreamBegin` handshake and switch modes. The dispatch
function splits at the connection level:

```rust
// After receiving a normal JSON request:
let response = dispatch(request, state.clone(), deps.clone()).await;
match response {
    DaemonResponse::StreamBegin { stream_id } => {
        // Switch this connection to framed streaming mode
        return handle_stream(stream_id, reader, writer, state, deps).await;
    }
    _ => {
        // Normal JSON response path (unchanged)
        send_json_response(&mut writer, response).await?;
    }
}
```

This keeps the streaming path isolated from the existing request-response loop.

---

## Phase 1 vs Phase 2 Scope

### Phase 1 (Maestro CLI sessions, no TTY)

- Exec command: `DaemonRequest::Exec { id, command, args }`
- Non-interactive: daemon runs command, accumulates output, returns single `DaemonResponse::ExecResult { stdout, stderr, exit_code }`
- **No streaming macro needed for Phase 1** — `handler_result!` macro (see `macro-handler-response.md`) is sufficient
- This is the path that unblocks `maestro start --runtime minibox`

### Phase 2 (PTY attach, browser terminal)

- Protocol switches to framed streaming after `StreamBegin` handshake
- `stream!` macro used for PTY forwarding
- Client sends `Stdin`/`Resize` frames; daemon forwards to PTY master fd
- Required for `execute_local_direct` (CLI terminal attach)

### Phase 3 (browser terminal)

- Requires bridge networking in minibox (out of scope for macro design)
- The streaming protocol itself is unchanged — just needs a container IP

---

## Why not use an existing streaming crate?

`tokio-stream`, `futures::Stream` — these handle async iteration but don't define the
wire protocol or enforce the terminal-frame invariant. The macro's value is encoding
minibox's specific streaming contract, not reimplementing async iteration.

---

## Related Docs

- `macro-command-registration.md` — dispatch table (Phase 1 commands use this)
- `macro-handler-response.md` — error wrapping for non-streaming handlers
- `docs/plans/maestro-minibox.md` — the two real blockers (exec, TTY)
