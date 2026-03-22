# Crate dependency graph

Shows the workspace crate relationships. Platform‑specific dependencies are gated at compile time — `macbox` is only compiled on macOS, `winbox` only on Windows. `daemonbox` and `minibox-lib` are platform‑agnostic and compiled everywhere. `minibox-cli` is also platform‑agnostic (it only speaks JSON over a socket/pipe).

## Mermaid

```mermaid
flowchart LR
    %% Nodes
    miniboxd["miniboxd\n(unified binary)"]
    minibox_cli["minibox-cli\n(platform-agnostic)"]
    minibox_bench["minibox-bench\n(platform-agnostic)"]

    macbox["macbox\n[cfg(target_os = \"macos\")]"]
    winbox["winbox\n[cfg(target_os = \"windows\")]"]
    daemonbox["daemonbox\n(platform-agnostic)"]

    minibox_lib["minibox-lib\n(platform-agnostic core)"]
    minibox_macros["minibox-macros\n(proc-macro)"]
    nix["nix\n[cfg(unix)]"]

    %% Edges (cfg shown as short labels)
    miniboxd -->|"linux"| minibox_lib
    miniboxd -->|"linux"| nix
    miniboxd -->|"macos"| macbox
    miniboxd -->|"windows"| winbox

    macbox --> daemonbox
    macbox --> minibox_lib

    winbox --> daemonbox
    winbox --> minibox_lib

    daemonbox --> minibox_lib

    minibox_cli --> minibox_lib
    minibox_bench --> minibox_lib

    minibox_lib --> minibox_macros

    %% Soft edge style for macro
    linkStyle 9 stroke-dasharray: 4 2

    %% Classes
    classDef binary fill:#e2e3e5,stroke:#6c757d,color:#000;
    classDef shim_macos fill:#d4edda,stroke:#155724,color:#000;
    classDef shim_windows fill:#cce5ff,stroke:#004085,color:#000;
    classDef shim_unix fill:#fff3cd,stroke:#856404,color:#000;
    classDef core fill:#f8d7da,stroke:#721c24,color:#000;
    classDef macro fill:#e2d9f3,stroke:#4b3c82,color:#000;
    classDef external fill:#f0f0f0,stroke:#999,color:#000,font-style:italic;

    class miniboxd,minibox_cli,minibox_bench binary;
    class macbox shim_macos;
    class winbox shim_windows;
    class daemonbox shim_unix;
    class minibox_lib core;
    class minibox_macros macro;
    class nix external;
```

## ASCII (fallback)

```text
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
│  minibox-macros  │  <--  used by minibox-lib
└──────────────────┘
```

## Getting started

```bash
# Check the workspace builds
cargo check

# Run the unified daemon
cargo run -p miniboxd

# Use the CLI against a running daemon
cargo run -p minibox-cli -- --help
```

For more detailed usage, flags, and configuration examples, see the individual crate READMEs under `crates/`.
```

Want a second `README-slides.md` variant with a darker background‑friendly Mermaid theme for your slide generator?
