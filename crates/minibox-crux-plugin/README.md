# minibox-crux-plugin

`minibox-crux-plugin` is a newline-delimited JSON stdio bridge between Crux workflows and a local
`miniboxd` daemon. It translates namespaced handler invocations into the canonical
`minibox_core::protocol::DaemonRequest` types and returns serialized daemon responses.

```text
Crux host <-> minibox-crux-plugin (stdin/stdout) <-> miniboxd (Unix socket)
```

The package provides both the `minibox-crux-plugin` binary and a `minibox_crux_plugin` library.

## Stdio protocol

Requests are tagged with `method`; responses are tagged with `status`. Each JSON object occupies
one line. Logs go to stderr so stdout remains protocol-only.

```json
{ "method": "Declare" }
```

```json
{
  "method": "Invoke",
  "params": { "handler": "minibox::container::ps", "input": {} }
}
```

```json
{ "method": "Shutdown" }
```

| Request                     | Response                                       |
| --------------------------- | ---------------------------------------------- |
| `Declare`                   | `Declare { handlers }`                         |
| `Invoke { handler, input }` | `InvokeOk { output }` or `InvokeErr { error }` |
| `Shutdown`                  | `ShutdownAck`, then process exit               |

Malformed JSON lines are logged and skipped rather than producing a protocol response.

## Handlers

| Handler                      | Required or notable input                                      |
| ---------------------------- | -------------------------------------------------------------- |
| `minibox::container::run`    | `image`; optional command, env, mounts, limits, name, platform |
| `minibox::container::stop`   | `id`                                                           |
| `minibox::container::pause`  | `id`                                                           |
| `minibox::container::resume` | `id`                                                           |
| `minibox::container::rm`     | `id`                                                           |
| `minibox::container::exec`   | `id`, `command`; optional env and tty                          |
| `minibox::container::ps`     | empty object                                                   |
| `minibox::container::logs`   | `id`                                                           |
| `minibox::image::pull`       | `image`; optional tag and platform                             |
| `minibox::image::build`      | `context_path`; optional tag and Dockerfile text               |
| `minibox::image::push`       | `image`                                                        |
| `minibox::image::ls`         | empty object                                                   |
| `minibox::image::rm`         | `image_ref`                                                    |

The run handler creates a non-ephemeral request. Exec is available only when the selected daemon
suite supplies an exec runtime. Push currently uses anonymous credentials and the `image` field as
the complete target reference; an advertised separate `target` value is not consumed.

## Build and run

```bash
cargo build -p minibox-crux-plugin --release
RUST_LOG=minibox_crux_plugin=debug ./target/release/minibox-crux-plugin
```

The daemon must be reachable through the standard socket resolution rules:
`MINIBOX_SOCKET_PATH`, `MINIBOX_RUN_DIR`, then the platform default.

## Library API

| API                                          | Purpose                                                             |
| -------------------------------------------- | ------------------------------------------------------------------- |
| `handler_decls()`                            | Return the 13 declared handler names and descriptions               |
| `build_request()`                            | Validate JSON input and construct a daemon request                  |
| `dispatch()`                                 | Call the daemon and collect responses through terminal/stream close |
| `process_request()`                          | Execute one stdio protocol request                                  |
| `parse_mounts()`                             | Validate absolute bind-mount paths without parent traversal         |
| `protocol::{Request, Response, HandlerDecl}` | Public stdio message model                                          |

## Security and limitations

- Bind-mount paths must be absolute and contain no `..` components.
- Mutations still pass through daemon policy and selected-adapter capability checks.
- Native daemon sockets additionally enforce peer credentials.
- This is a local stdio/Unix-socket bridge; it provides no remote authentication or transport.
- Handler inputs do not yet expose every field in the daemon protocol.

## Development and testing

Unit tests cover every request mapping and validation path. Integration tests spawn the binary,
exercise declare/invoke/shutdown framing, and use a mock daemon to verify translated requests and
streaming responses.

```bash
cargo check -p minibox-crux-plugin
cargo clippy -p minibox-crux-plugin --all-targets -- -D warnings
cargo nextest run -p minibox-crux-plugin
```
