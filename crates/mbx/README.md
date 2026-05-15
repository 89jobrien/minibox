# mbx

`mbx` is the CLI client for the miniboxd container runtime.

Connects to the miniboxd daemon over its Unix socket and issues JSON-over-newline requests,
printing human-readable output.

## Commands

```
mbx pull <image> [--tag TAG] [--platform PLATFORM]
mbx run [OPTIONS] <image> -- <command>
mbx ps
mbx stop <id>
mbx rm [<id> | --all]
mbx pause <id>
mbx resume <id>
mbx exec [--tty] [-i] [-u USER] <id> -- <command>
mbx logs [--follow] <id>
mbx events
mbx prune [--dry-run]
mbx rmi <image:tag>
mbx update [--all] [--containers] [--restart] [<images>...]
mbx upgrade [--dry-run] [--version VERSION]
mbx load [--name NAME] [--tag TAG] <path>
mbx sandbox [OPTIONS] <script>
mbx snapshot save <id> [--name NAME]
mbx snapshot restore <id> <name>
mbx snapshot list <id>
mbx diagnose <id>
mbx doctor
```

### `run` flags

```
--memory N          Memory limit in bytes (cgroups v2 memory.max)
--cpu-weight N      CPU weight 1-10000 (cgroups v2 cpu.weight)
--tag TAG           Image tag (default: latest)
--network MODE      none (default) | bridge | host | tailnet
--privileged        Grant full Linux capabilities
-v SRC:DST[:ro]     Bind mount (repeatable)
--mount type=bind,src=PATH,dst=PATH[,readonly]
--name NAME         Assign a human-readable name
--tty / -i          Allocate PTY / keep stdin open
-e KEY=VALUE        Set environment variables (repeatable)
--entrypoint CMD    Override image entrypoint
-u USER             Run as user (e.g. nobody, 1000:1000)
--rm                Remove container on exit
--platform PLATFORM Target platform (e.g. linux/arm64)
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
