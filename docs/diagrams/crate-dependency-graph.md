# Crate Dependency Graph

Shows the workspace crate relationships. Platform-specific dependencies are gated at compile
time — `macbox` is only compiled on macOS, `winbox` only on Windows. `daemonbox` and
`minibox-core` are platform-agnostic. `minibox` is a thin re-export facade over `linuxbox`,
which contains the actual Linux adapter implementations. New in 2026-Q2: `searchbox`,
`zoektbox`, `tailbox`, `minibox-agent`, `dashbox`, `dockerbox`, `minibox-secrets`, `minibox-llm`.

## Mermaid

```mermaid
flowchart LR
    subgraph binaries["Binaries"]
        miniboxd["miniboxd\n(unified daemon)"]
        mbx["mbx\n(CLI binary)"]
        minibox_bench["minibox-bench"]
        miniboxctl["miniboxctl\n(HTTP API, WIP)"]
        dockerboxd["dockerboxd\n(Docker shim)"]
        dashbox["dashbox\n(TUI dashboard)"]
        searchboxd["searchboxd\n(MCP stdio server)"]
    end

    subgraph platform["Platform Crates"]
        macbox["macbox\n[cfg(macos)]"]
        winbox["winbox\n[cfg(windows)]"]
    end

    subgraph daemon["Daemon"]
        daemonbox["daemonbox\n(platform-agnostic\nhandler + state + server)"]
    end

    subgraph core["Core"]
        minibox_core["minibox-core\n(protocol + domain traits)"]
        minibox["minibox\n(re-export facade\nover linuxbox)"]
        linuxbox["linuxbox\n(Linux adapters)"]
        minibox_macros["minibox-macros\n(proc-macro)"]
        minibox_oci["minibox-oci\n(OCI types)"]
    end

    subgraph services["Service / Feature Crates"]
        searchbox["searchbox\n(SearchProvider port + adapters)"]
        zoektbox["zoektbox\n(Zoekt lifecycle)"]
        tailbox["tailbox\n(Tailnet adapter)"]
        dockerbox["dockerbox\n(Docker API shim lib)"]
        minibox_agent["minibox-agent\n(AI agent runtime)"]
        minibox_llm["minibox-llm\n(LLM client)"]
        minibox_secrets["minibox-secrets\n(credential store)"]
    end

    subgraph devtools["Dev / Test"]
        minibox_testers["minibox-testers\n(shared test helpers)"]
        nix["nix\n[cfg(unix)]"]
    end

    miniboxd -->|"linux"| linuxbox
    miniboxd -->|"linux"| nix
    miniboxd -->|"macos"| macbox
    miniboxd -->|"windows"| winbox
    miniboxd --> daemonbox
    miniboxd -->|"tailnet feature"| tailbox

    macbox --> daemonbox
    macbox --> minibox_core
    winbox --> daemonbox
    winbox --> minibox_core

    daemonbox --> minibox_core

    minibox --> linuxbox
    linuxbox --> minibox_core
    linuxbox --> minibox_oci
    linuxbox --> minibox_macros

    mbx --> minibox_core
    minibox_bench --> linuxbox
    miniboxctl --> minibox_core
    dockerboxd --> dockerbox
    dockerbox --> minibox_core

    searchboxd --> searchbox
    searchbox --> zoektbox
    searchbox --> minibox_core

    minibox_agent --> minibox_llm
    minibox_agent --> minibox_secrets
    minibox_agent --> minibox_core
    tailbox --> minibox_secrets
    tailbox --> minibox_core

    classDef binary fill:#e2e3e5,stroke:#6c757d,color:#000;
    classDef platform_mac fill:#d4edda,stroke:#155724,color:#000;
    classDef platform_win fill:#cce5ff,stroke:#004085,color:#000;
    classDef daemon_crate fill:#fff3cd,stroke:#856404,color:#000;
    classDef core_crate fill:#f8d7da,stroke:#721c24,color:#000;
    classDef macro_crate fill:#e2d9f3,stroke:#4b3c82,color:#000;
    classDef service_crate fill:#fde8d8,stroke:#a0522d,color:#000;
    classDef new_crate fill:#ffe0b2,stroke:#e65100,color:#000;
    classDef external fill:#f0f0f0,stroke:#999,color:#000,font-style:italic;

    class miniboxd,mbx,minibox_bench,miniboxctl,dockerboxd,dashbox,searchboxd binary;
    class macbox platform_mac;
    class winbox platform_win;
    class daemonbox daemon_crate;
    class minibox_core,minibox,linuxbox,minibox_oci core_crate;
    class minibox_macros macro_crate;
    class minibox_agent,minibox_llm,minibox_secrets,dockerbox service_crate;
    class searchbox,zoektbox,tailbox new_crate;
    class minibox_testers,nix external;
```

## ASCII

```
BINARIES
  miniboxd ──[linux]──► linuxbox ──► minibox-core
           ──[linux]──► nix
           ──[macos]──► macbox ──► daemonbox ──► minibox-core
           ──[win]────► winbox ──► daemonbox
           ────────────► daemonbox
           ──[tailnet]──► tailbox ──► minibox-secrets ──► minibox-core
  mbx (CLI binary), miniboxctl ───────────────────────► minibox-core
  minibox-bench ──────────────────────────────────────► linuxbox
  dockerboxd ──► dockerbox ──────────────────────────► minibox-core
  searchboxd ──► searchbox ──► zoektbox
                           ──► minibox-core

CORE
  minibox-core  ← canonical protocol + domain traits
  linuxbox      ← Linux adapters (namespaces, cgroups, overlay, OCI)
  minibox       ← thin re-export facade: `pub use linuxbox::*`
  minibox-oci   ← OCI image types
  minibox-macros ← proc-macros (as_any!, adapt!)

SERVICE / FEATURE (2026-Q2 additions marked *)
  searchbox*    ← SearchProvider port + Zoekt/fan-out/fs adapters
  zoektbox*     ← Zoekt binary lifecycle (download, verify, deploy)
  tailbox*      ← Tailnet/Tailscale NetworkProvider adapter
  minibox-agent ← AI agent runtime (wired to crux)
  minibox-llm   ← Multi-provider LLM client
  minibox-secrets ← Credential store (env/keyring/1Password/Bitwarden)
  dockerbox     ← Docker API shim library
```
