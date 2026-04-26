# Dagu Integration

This document describes how dagu (workflow engine) integrates with minibox (container runtime)
and clarifies the dependency model. Addresses GH #31.

## dagu is an external dependency, not a git submodule

dagu is consumed as a pre-built container image from `ghcr.io/dagu-org/dagu:latest`.  There is
no `.gitmodules` entry and no nested dagu checkout in this repository — that is intentional.

The integration components in this repository are:

| Path | Description |
|------|-------------|
| `minibox-dagu/Dockerfile` | Builds the `ghcr.io/89jobrien/mbx-dagu` image on top of the official dagu base image. |
| `minibox-dagu/executor.go` | The `mbx-dagu` binary: translates dagu step definitions to miniboxctl HTTP API calls. |
| `crates/miniboxctl/` | HTTP controller that bridges dagu → miniboxd over the Unix socket. |
| `docs/archive/dagu-minibox-integration.md` | Architecture diagrams (Mermaid + ASCII). **(archived)** |
| `docs/superpowers/specs/2026-03-24-dagu-minibox-orchestration-design.md` | Full design spec. |

## Runtime dependency chain

```
dagu workflow engine   (ghcr.io/dagu-org/dagu:latest — external, not vendored)
        |
        | invokes
        v
mbx-dagu executor      (built into ghcr.io/89jobrien/mbx-dagu via minibox-dagu/Dockerfile)
        |
        | POST /api/v1/jobs   (HTTP, localhost:9999)
        v
miniboxctl             (crates/miniboxctl — Rust binary, runs on host)
        |
        | DaemonRequest::Run  (JSON over Unix socket)
        v
miniboxd               (crates/miniboxd — Rust daemon, requires root/Linux)
```

## Building the image

```bash
cd minibox-dagu
docker build -t ghcr.io/89jobrien/mbx-dagu:latest .
```

The build requires Docker (or compatible OCI builder).  dagu itself is pulled from
`ghcr.io/dagu-org/dagu:latest` at build time — no local dagu installation is needed.

## Running a dagu workflow with minibox

```bash
# 1. Start miniboxctl on the host (binds localhost:9999 by default)
./target/release/miniboxctl --listen localhost:9999 &

# 2. Run the dagu container
minibox run ghcr.io/89jobrien/mbx-dagu:latest \
  -e MBXCTL_URL=http://localhost:9999 \
  -- server

# 3. Open the dagu Web UI
open http://localhost:8080
```

## Resource limits

The `mbx-dagu` executor accepts `--memory` and `--cpu-weight` flags and passes them to
miniboxctl as `memory_limit_bytes` and `cpu_weight` on the `CreateJobRequest`.  These map
directly to cgroups v2 `memory.max` and `cpu.weight` on the Linux host.

Example DAG step with limits:

```yaml
steps:
  - name: build
    command: mbx-dagu --image rust --tag 1.77 --memory 2147483648 --cpu-weight 512 -- cargo build
```

## Version pinning

To pin a specific dagu version, change the `FROM` line in `minibox-dagu/Dockerfile`:

```dockerfile
FROM ghcr.io/dagu-org/dagu:1.14.5
```

Check available tags at https://github.com/dagu-org/dagu/releases.

## Security

miniboxctl binds `localhost:9999` by default.  When dagu runs in a container, set
`MBXCTL_URL=http://host-gateway:9999` (Docker) or use a shared network namespace.  Never
expose miniboxctl on `0.0.0.0` without a reverse proxy with authentication.  See
`docs/SECURITY.md` for details.
