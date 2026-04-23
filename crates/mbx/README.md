# minibox-cli

CLI client binary (`minibox` command) for managing containers via the daemon.

## Commands

- `minibox pull <image>` — Fetch image from Docker Hub
- `minibox run [--memory M] [--cpus N] <image> -- <command>` — Run container (ephemeral by default)
- `minibox ps` — List running containers
- `minibox stop <container-id>` — Stop container
- `minibox rm <container-id>` — Remove container

## Ephemeral Mode

`minibox run` streams container stdout/stderr in real time and exits with the container's exit code. The container is automatically cleaned up on completion.

## Socket Communication

CLI sends requests to the daemon via the Unix socket (`/run/minibox/miniboxd.sock`). Responses are JSON-over-newline. For streaming containers, output is received via broadcast channels.

## Testing

Enable the `subprocess-tests` feature to run CLI integration tests against a pre-built `minibox` binary.
