# minibox-crux-plugin

JSON-RPC stdio plugin that exposes minibox container and image operations
to [crux](https://github.com/89jobrien/crux) agents.

## How it works

The plugin runs as a standalone binary, communicating over stdin/stdout
using the crux plugin protocol (newline-delimited JSON). It connects to
a running `miniboxd` daemon via its Unix socket and translates crux
handler invocations into `DaemonRequest`/`DaemonResponse` round-trips.

```
crux agent  <-->  minibox-crux-plugin (stdio)  <-->  miniboxd (Unix socket)
```

## Handlers

13 handlers across two namespaces:

| Handler | Description |
|---|---|
| `minibox::container::run` | Create and start a container |
| `minibox::container::stop` | Stop a running container |
| `minibox::container::pause` | Freeze a container (cgroup.freeze) |
| `minibox::container::resume` | Thaw a paused container |
| `minibox::container::rm` | Remove a stopped container |
| `minibox::container::exec` | Execute a command in a running container |
| `minibox::container::ps` | List all containers |
| `minibox::container::logs` | Fetch container logs |
| `minibox::image::pull` | Pull an image from a registry |
| `minibox::image::build` | Build an image from a Dockerfile |
| `minibox::image::push` | Push an image to a registry |
| `minibox::image::ls` | List cached images |
| `minibox::image::rm` | Remove a cached image |

## Usage

```bash
# Build
cargo build -p minibox-crux-plugin --release

# Run (miniboxd must be running)
./target/release/minibox-crux-plugin
```

The plugin reads `Request` objects from stdin and writes `Response`
objects to stdout. Logging goes to stderr (controlled by `RUST_LOG`).

## Protocol messages

| Request | Response |
|---|---|
| `Declare` | `Declare { handlers }` |
| `Invoke { handler, input }` | `InvokeOk { output }` or `InvokeErr { error }` |
| `Shutdown` | `ShutdownAck` |

## Security

Mount inputs are validated: paths must be absolute with no `..`
components. The plugin itself does not perform container operations
directly — all mutations go through `miniboxd`, which enforces its own
auth (SO_PEERCRED) and policy gates.
