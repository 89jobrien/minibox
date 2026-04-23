# mbx

`mbx` is the CLI client for the miniboxd container runtime.

Connects to the miniboxd daemon over its Unix socket and issues JSON-over-newline requests,
printing human-readable output.

## Commands

```
mbx pull <image>                              # Pull image from Docker Hub
mbx run [--memory M] [--cpus N] <image> -- <command>  # Run container
mbx ps                                        # List containers
mbx stop <container-id>                       # Stop a container
mbx rm <container-id>                         # Remove a container
mbx exec <container-id> -- <command>          # Exec into a container
mbx images                                    # List local images
```

## Ephemeral mode

`mbx run` uses `ephemeral: true` and streams container stdout/stderr in real time until a
`ContainerStopped` frame arrives. The CLI then exits with the container's reported exit code.

## Socket communication

Requests are serialised as a single JSON line (`DaemonRequest`) and written to the Unix socket
at `/run/minibox/miniboxd.sock` (override with `MINIBOX_SOCKET_PATH`). Each response is a
newline-delimited `DaemonResponse` JSON object.

## Features

| Feature            | Description                                                   |
| ------------------ | ------------------------------------------------------------- |
| `subprocess-tests` | Enable integration tests that spawn a real `miniboxd` binary. |
|                    | Run via `just test-cli-subprocess`.                           |

## Building

```bash
cargo build -p mbx --release
# output: target/release/mbx
```
