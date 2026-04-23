# Crate Dependency Graph

Shows the workspace crate relationships. Platform-specific dependencies are gated at compile
time — `macbox` is only compiled on macOS, `winbox` only on Windows. `daemonbox` and `minibox`
are platform-agnostic. New in 2026-Q2: `searchbox`, `zoektbox`, `tailbox`, `minibox-agent`,
`dashbox`, `dockerbox`, `minibox-secrets`, `minibox-llm`.

## Mermaid

```mermaid
flowchart LR
    subgraph binaries["Binaries"]
        miniboxd["miniboxd\n(unified daemon)"]
        minibox_cli["minibox-cli"]
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
        minibox["minibox\n(Linux adapters)"]
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

    miniboxd -->|"linux"| minibox
    miniboxd -->|"linux"| nix
    miniboxd -->|"macos"| macbox
    miniboxd -->|"windows"| winbox
    miniboxd --> daemonbox
    miniboxd --> tailbox

    macbox --> daemonbox
    macbox --> minibox_core
    winbox --> daemonbox
    winbox --> minibox_core

    daemonbox --> minibox_core
    daemonbox --> minibox_agent

    minibox --> minibox_core
    minibox --> minibox_oci
    minibox --> minibox_macros

    minibox_cli --> minibox_core
    minibox_bench --> minibox_core
    miniboxctl --> minibox_core
    dockerboxd --> dockerbox
    dockerbox --> minibox_core

    searchboxd --> searchbox
    searchbox --> zoektbox
    searchbox --> minibox_core

    dashbox --> minibox_core

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

    class miniboxd,minibox_cli,minibox_bench,miniboxctl,dockerboxd,dashbox,searchboxd binary;
    class macbox platform_mac;
    class winbox platform_win;
    class daemonbox daemon_crate;
    class minibox_core,minibox,minibox_oci core_crate;
    class minibox_macros macro_crate;
    class minibox_agent,minibox_llm,minibox_secrets,dockerbox service_crate;
    class searchbox,zoektbox,tailbox new_crate;
    class minibox_testers,nix external;
```

## ASCII

```
BINARIES
  miniboxd ──[linux]──► minibox ──► minibox-core
           ──[linux]──► nix
           ──[macos]──► macbox ──► daemonbox ──► minibox-core
           ──[win]────► winbox ──► daemonbox
           ────────────► daemonbox
           ────────────► tailbox ──► minibox-secrets ──► minibox-core
  minibox-cli, minibox-bench, miniboxctl ──────────────► minibox-core
  dockerboxd ──► dockerbox ──────────────────────────► minibox-core
  searchboxd ──► searchbox ──► zoektbox
                           ──► minibox-core
  dashbox ───────────────────────────────────────────► minibox-core

CORE
  minibox-core  ← canonical protocol + domain traits
  minibox       ← Linux adapters; re-exports minibox-core
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
