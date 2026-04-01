# mbx-dagu

A CLI bridge that lets [dagu](https://github.com/dagu-org/dagu) run containers via minibox.

## Architecture

`mbx-dagu` is a standalone Go binary (no dagu import) that:
1. Accepts container configuration as CLI flags
2. Submits a job to `mbxctl` (the minibox control plane) via HTTP
3. Polls until the job completes
4. Exits with the container's exit code

It is used as a dagu step via the built-in `command` executor type — no dagu plugin
registration required.

```
dagu step  ──(command executor)──►  mbx-dagu  ──(HTTP)──►  mbxctl  ──(Unix socket)──►  miniboxd
```

## Usage in a dagu workflow

```yaml
steps:
  - name: run-alpine
    command: mbx-dagu --image alpine --tag latest -- /bin/echo hello

  - name: with-resources
    command: mbx-dagu --image alpine --memory 134217728 --cpu-weight 512 -- /bin/sh -c "echo ok"

  - name: with-env
    command: mbx-dagu --image alpine --env MY_VAR=hello,OTHER_VAR=world -- /bin/sh -c 'echo $MY_VAR'
```

See `example-workflow.yaml` for a complete example.

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--image` | (required) | Container image name |
| `--tag` | `latest` | Image tag |
| `--memory` | `0` | Memory limit in bytes (0 = unlimited) |
| `--cpu-weight` | `0` | CPU weight (0 = default) |
| `--env` | `""` | Comma-separated `KEY=VALUE` env vars |
| `--timeout` | `1h` | Job timeout |
| `--mbxctl` | `$MBXCTL_URL` or `http://localhost:9999` | mbxctl base URL |

## Directory layout

This directory is part of the minibox workspace (not a git submodule). It lives at
`mbx-dagu/` in the minibox repo root. It has its own `go.mod` because it is a
separate Go module with no Rust/cargo dependencies.

```
mbx-dagu/
  cmd/mbx-dagu/main.go        — CLI entry point (flag parsing, job lifecycle)
  internal/client/client.go   — HTTP client for mbxctl /jobs API
  internal/client/models.go   — Request/response types
  Dockerfile                  — Build mbx-dagu and layer onto dagu base image
  example-workflow.yaml       — Working dagu workflow example
```

## Building

```bash
# Local build
cd mbx-dagu
go build -o mbx-dagu ./cmd/mbx-dagu

# Docker image (dagu + mbx-dagu)
docker build -t mbx-dagu:latest .
```

## Prerequisites

- `miniboxd` daemon running on the host
- `mbxctl` running: `mbxctl --listen localhost:9999`
- mbx-dagu binary in PATH (or use the Docker image)
