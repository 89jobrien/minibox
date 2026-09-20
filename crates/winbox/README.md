# winbox

`winbox` is the Windows platform boundary for minibox. It is currently a Phase 1 stub: the crate
compiles its intended public surface, but it cannot start a daemon or run a container.

The package is internal to the workspace (`publish = false`). On Windows, `miniboxd` calls
`winbox::start()`, which currently returns an error unconditionally.

## Current API

| API                       | Current behavior                                                   |
| ------------------------- | ------------------------------------------------------------------ |
| `start()`                 | Logs startup and returns "Windows server loop not yet implemented" |
| `hcs::start_container()`  | Always returns an HCS Phase 2 error                                |
| `wsl2::start_container()` | Always returns a WSL2 Phase 2 error                                |
| `preflight()`             | Probes Windows Containers and WSL2 through an injectable executor  |
| `WinboxStatus`            | Reports HCS, WSL2, both, or no backend                             |
| `paths::data_dir()`       | Returns the planned Windows data directory                         |
| `paths::run_dir()`        | Returns the planned ephemeral runtime directory                    |
| `paths::pipe_name()`      | Returns `\\.\pipe\miniboxd`                                        |

The preflight API can be tested without invoking PowerShell or WSL:

```rust
use winbox::preflight::{Executor, WinboxStatus, preflight};

let executor: Executor = Box::new(|args| {
    match args.first().copied() {
        Some("powershell") => Ok("Enabled".to_string()),
        _ => Err(anyhow::anyhow!("not installed")),
    }
});

assert_eq!(preflight(&executor), WinboxStatus::Hcs);
```

## Not implemented

- Named Pipe listener and protocol transport;
- Named Pipe ACL or token authentication;
- HCS compute-system lifecycle;
- WSL2 image import and process lifecycle;
- Windows OCI image management;
- adapter composition into `HandlerDependencies`; and
- Windows end-to-end daemon/CLI tests.

The similarly named HCS and WSL2 types in `minibox::adapters` are also library stubs. They are not
evidence of a working Windows daemon.

## Platform constraints

The crate can be compiled on non-Windows hosts for contract tests, but its default executor only
makes operational sense on Windows. The current `start()` path fails on every platform.

## Development and testing

The conformance stub test pins the fail-closed behavior so unfinished backends are not accidentally
reported as usable.

```bash
cargo check -p winbox
cargo clippy -p winbox --all-targets -- -D warnings
cargo nextest run -p winbox
```

When implementation begins, add Windows-native CI and transport tests before changing the support
status in the feature matrix.
