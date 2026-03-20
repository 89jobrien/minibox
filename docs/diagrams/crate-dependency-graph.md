# Crate Dependency Graph

## Description

Shows the workspace crate relationships. Platform-specific dependencies are gated
at compile time — `macbox` is only compiled on macOS, `winbox` only on Windows.
`daemonbox` and `minibox-lib` are platform-agnostic and compiled everywhere.
`minibox-cli` is also platform-agnostic (it only speaks JSON over a socket/pipe).

## ASCII

```
┌─────────────┐  [linux]  ┌───────────────┐
│             ├──────────►│  minibox-lib  │
│             │  [linux]  └───────────────┘
│             ├──────────►  nix
│  miniboxd   │           ┌───────────────┐  ┌────────────┐  ┌───────────────┐
│  (unified   │  [macos]  │    macbox     ├─►│ daemonbox  ├─►│  minibox-lib  │
│   binary)   ├──────────►│               ├─►└────────────┘  └───────────────┘
│             │           └───────────────┘
│             │           ┌───────────────┐  ┌────────────┐  ┌───────────────┐
│             │  [win]    │    winbox     ├─►│ daemonbox  ├─►│  minibox-lib  │
│             ├──────────►│               ├─►└────────────┘  └───────────────┘
└─────────────┘           └───────────────┘

┌─────────────────┐
│   minibox-cli   ├──────────────────────────────────────────►  minibox-lib
└─────────────────┘

┌──────────────────┐
│   minibox-bench  ├─────────────────────────────────────────►  minibox-lib
└──────────────────┘

┌──────────────────┐
│  minibox-macros  │  (proc-macro, used by minibox-lib)
└──────────────────┘
```

## Mermaid

```mermaid
graph TD
    miniboxd["miniboxd\n(unified binary)"]
    macbox["macbox\n[cfg(target_os=macos)]"]
    winbox["winbox\n[cfg(target_os=windows)]"]
    daemonbox["daemonbox\n(platform-agnostic)"]
    minibox_lib["minibox-lib\n(platform-agnostic)"]
    minibox_cli["minibox-cli"]
    minibox_macros["minibox-macros\n(proc-macro)"]
    nix["nix\n[cfg(unix)]"]

    miniboxd -->|"#[cfg(linux)]"| minibox_lib
    miniboxd -->|"#[cfg(linux)]"| nix
    miniboxd -->|"#[cfg(macos)]"| macbox
    miniboxd -->|"#[cfg(windows)]"| winbox

    macbox --> daemonbox
    macbox --> minibox_lib

    winbox --> daemonbox
    winbox --> minibox_lib

    daemonbox --> minibox_lib

    minibox_lib --> minibox_macros

    minibox_cli --> minibox_lib

    style macbox fill:#d4edda
    style winbox fill:#cce5ff
    style daemonbox fill:#fff3cd
    style minibox_lib fill:#f8d7da
```
