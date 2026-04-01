# dockerbox

HTTP-over-Unix-socket shim that translates Docker API calls to the minibox protocol. Allows Docker clients (`docker`, `docker-compose`, etc.) to interact with miniboxd without modification.

## Architecture

`dockerboxd` listens on a Unix socket and forwards requests to miniboxd, translating between the Docker v2 HTTP API and the minibox JSON-over-newline protocol.

- **Socket**: `/run/dockerbox/dockerbox.sock` (override with `DOCKERBOX_SOCKET`)
- **Upstream**: `MINIBOX_SOCKET` (default `/run/minibox/minibox.sock`)

## API Coverage

| Endpoint | Status |
|----------|--------|
| `POST /containers/create` | Implemented |
| `POST /containers/{id}/start` | No-op (minibox starts at `Run` time) |
| `GET /containers/json` | Implemented |
| `GET /containers/{id}/json` | Implemented |
| `DELETE /containers/{id}` | Implemented |
| `GET /containers/{id}/logs` | Stub |
| `GET /images/json` | Implemented |
| `POST /images/create` (pull) | Implemented |
| `GET /networks` | In-memory stub |
| `GET /volumes` | Stub (maps to `~/.local/share/dockerbox/volumes/`) |
| `GET /_ping` | Implemented |
| `GET /info` | Implemented |

## ID Translation

minibox uses 16-character hex container IDs; Docker expects 64. `dockerboxd` pads IDs with trailing zeros for compatibility.

## Socket Security

- Default permissions: `0o660` (root-owned, group-accessible)
- Set `DOCKERBOX_SOCKET_GROUP=docker` to allow group members to connect without sudo
- Set `DOCKERBOX_SOCKET_MODE=0640` to restrict permissions further
- All operations reaching miniboxd are still gated by `SO_PEERCRED` (UID 0 only)

## Running

```bash
sudo ./target/release/dockerboxd

# With custom sockets
DOCKERBOX_SOCKET=/tmp/docker.sock MINIBOX_SOCKET=/run/minibox/minibox.sock sudo ./target/release/dockerboxd

# Point Docker CLI at dockerbox
docker -H unix:///run/dockerbox/dockerbox.sock ps
```
