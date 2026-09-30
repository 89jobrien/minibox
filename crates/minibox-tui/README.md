# minibox-tui

`minibox-tui` is the read-only terminal dashboard for a running `miniboxd` daemon. It renders a
live container table and a bounded lifecycle-event log with Ratatui and Crossterm.

The crate is a library, not a standalone binary. The supported user entry point is the
feature-gated `mbx tui` command.

## Purpose and boundaries

The dashboard intentionally sends no mutating daemon requests. Container creation, stop, remove,
and exec remain in the primary `mbx` CLI, where the existing policy checks apply.

The event loop uses:

- `DaemonRequest::List` once per second for the container table;
- `DaemonRequest::SubscribeEvents` for live lifecycle events;
- Crossterm input events for navigation and exit; and
- `DaemonClient` socket resolution from `MINIBOX_SOCKET_PATH`, `MINIBOX_RUN_DIR`, or the
  platform default.

## Build and run

Enable the `tui` feature on the `minibox-cli` package:

```bash
cargo build -p minibox-cli --features tui
./target/debug/mbx tui
```

`miniboxd` must already be running and reachable through the normal daemon socket rules.

## Controls

| Key          | Action                        |
| ------------ | ----------------------------- |
| `q` or `Esc` | Quit                          |
| `j` or Down  | Select the next container     |
| `k` or Up    | Select the previous container |

The table displays container ID, optional name, image, state, and PID. The event pane keeps the
most recent 200 formatted events. Daemon connection or polling failures appear in the status bar.

## Rust API

Most consumers only need the top-level runner:

```rust,no_run
#[tokio::main]
async fn main() -> miette::Result<()> {
    minibox_tui::run().await
}
```

The public modules also expose:

| API                         | Purpose                                            |
| --------------------------- | -------------------------------------------------- |
| `app::App`                  | Dashboard state and selection/event transitions    |
| `event::Message`            | Input, timer, list, event, and error messages      |
| `event::apply`              | Apply a message to `App`                           |
| `event::refresh_containers` | Fetch the current daemon container list            |
| `ui::draw`                  | Render the complete dashboard into a Ratatui frame |

`run()` enables raw mode and the alternate screen. After the event loop returns, including with an
error, it attempts to disable raw mode and then leave the alternate screen before returning.
Initialization failures, cleanup failures, or panics can still interrupt that best-effort cleanup.

## Constraints

- Requires an interactive terminal.
- Read-only by design; there are no run, stop, remove, or exec actions.
- Event subscription errors are reported but are not automatically reconnected.
- Container polling is fixed at a one-second interval.
- The crate has no Cargo features of its own; availability is controlled by `minibox-cli`'s `tui`
  feature.

## Development and testing

Inline tests cover selection wrapping, shrinking lists, empty state, bounded event history, and
message application. A live smoke test requires a running daemon.

```bash
cargo check -p minibox-tui
cargo clippy -p minibox-tui --all-targets -- -D warnings
cargo nextest run -p minibox-tui
cargo build -p minibox-cli --features tui
```
