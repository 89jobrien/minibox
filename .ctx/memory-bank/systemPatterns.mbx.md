# System patterns

## High-level layout

11-crate Rust 2024 workspace (v0.30.0):

```
minibox-macros       proc-macro (~300 LOC)
minibox-core         cross-platform types, domain traits, protocol (~12.6k LOC)
minibox              Linux adapters, daemon handler/server/state (~21.5k LOC)
macbox               macOS backend wiring (delegates to smolbox)
smolbox              smolvm + krun adapter implementations
winbox               Windows stub (WSL2, ~40% scaffolded)
miniboxd             daemon entry point, adapter DI composition root (~1.6k LOC)
mbx                  CLI client (~3.2k LOC)
minibox-crux-plugin  crux plugin host (JSON-RPC stdio)
minibox-testsuite    conformance test harness
xtask                CI gates, test runners, bench (~5k LOC)
```

## Data flow

1. CLI (`mbx`) sends `DaemonRequest` over Unix socket
2. `miniboxd` deserializes, routes to handler
3. Handler calls domain trait methods on injected adapter suite
4. Adapter suite implements traits (native/gke/colima/smolvm/krun)
5. Handler sends `DaemonResponse` back over socket

## Patterns to follow

- **Hexagonal architecture**: domain traits in `minibox-core/src/domain.rs`,
  adapters in `minibox/src/adapters/`
- **Error handling**: `anyhow::Context` everywhere, no `.unwrap()` in production
- **Tracing**: structured `key = value` fields, never embedded in message string
- **Path validation**: all external paths through `validate_layer_path()` before
  filesystem ops
- **Async/sync boundary**: `spawn_blocking` for fork/clone/exec, never inline
- **Protocol changes**: start in `minibox-core/src/protocol.rs`, update handlers
  + CLI + snapshots together. New fields get `#[serde(default)]`.
- **`HandlerDependencies` changes**: update ALL adapter suite construction sites
  in `miniboxd/src/main.rs`
- **Testing**: `expect("reason")` in tests, never bare `.unwrap()`
- **Unsafe**: every `unsafe {}` block requires a `// SAFETY:` comment

## Patterns to avoid

- `println!`/`eprintln!` in daemon code (use tracing)
- `Path::join(user_input)` without validation (Zip Slip)
- `fork()`/`clone()` in async fn (blocks tokio)
- `let _ = send(...)` on handler channels (log dropped-client cases)
- `set_var`/`remove_var` without static Mutex guard (Rust 2024 = unsafe)
- `OwnedFd` alive across `clone()` (double-close)

## Git workflow

`develop` -> `next` -> `staging` -> `main` -> `v*` tag
