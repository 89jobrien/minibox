# mbxctl

Alternative management CLI for minibox with an Axum-based HTTP/SSE API. Provides job tracking, structured container lifecycle management, and a server-sent events stream for real-time output.

## Status

Work in progress. Core job execution and SSE streaming are implemented; full feature parity with `minibox-cli` is incomplete.

## Architecture

`mbxctl` runs an Axum HTTP server that proxies requests to miniboxd via the Unix socket client. Container runs are tracked as "jobs" with persistent log history.

## API

| Method   | Path              | Description                      |
| -------- | ----------------- | -------------------------------- |
| `POST`   | `/jobs`           | Create and start a container job |
| `GET`    | `/jobs`           | List all jobs                    |
| `GET`    | `/jobs/{id}`      | Get job status                   |
| `GET`    | `/jobs/{id}/logs` | SSE stream of job output         |
| `DELETE` | `/jobs/{id}`      | Stop and remove a job            |

## Running

```bash
./target/release/mbxctl

# Custom listen address and minibox socket
mbxctl --listen 127.0.0.1:8080 --socket /run/minibox/minibox.sock
```
