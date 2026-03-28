# Environment Variable Flow

Traces how user-supplied env vars travel from the CLI or mbxctl HTTP API
all the way to `execve(2)` inside the container process.

## Mermaid

```mermaid
flowchart TD
    CLI["minibox-cli\nrun.rs\nDaemonRequest::Run { env: vec![] }"]
    mbxctl["mbxctl\nCreateJobRequest { env: [\"FOO=bar\"] }\n→ DaemonRequest::Run { env }"]

    protocol["Protocol layer\nminibox-core/protocol.rs\nDaemonRequest::Run {\n  env: Vec&lt;String&gt;  // #[serde(default)]\n}"]

    server["daemonbox/server.rs\ndispatch() destructures env\npasses to handle_run()"]

    handler["daemonbox/handler.rs\nhandle_run(env)\n  → handle_run_streaming(env)\n    → run_inner_capture(env)\n      container_env = [PATH, TERM] + env\n  → run_inner(env)\n      container_env = [PATH, TERM] + env"]

    spawnconfig["ContainerSpawnConfig { env: container_env }"]

    adapter["Runtime adapter\nspawn_process(config)\n  → spawn_blocking(child_init)"]

    child["child_init()\nBuild envp: Vec&lt;CString&gt;\nfrom config.env"]

    execve["execve(cmd, argv, envp)\nContainer process starts\nwith explicit environment"]

    CLI --> protocol
    mbxctl --> protocol
    protocol --> server
    server --> handler
    handler --> spawnconfig
    spawnconfig --> adapter
    adapter --> child
    child --> execve

    classDef proto fill:#fff3cd,stroke:#856404,color:#000;
    classDef handler fill:#d4edda,stroke:#155724,color:#000;
    classDef kernel fill:#f8d7da,stroke:#721c24,color:#000;
    classDef input fill:#cce5ff,stroke:#004085,color:#000;

    class CLI,mbxctl input;
    class protocol,server proto;
    class handler,spawnconfig,adapter,child handler;
    class execve kernel;
```

## ASCII (fallback)

```text
  minibox-cli              mbxctl HTTP API
  run.rs                   POST /jobs {"env":["FOO=bar"]}
  env: vec![]                    │
       │                         │
       └──────────┬──────────────┘
                  │
                  ▼
         DaemonRequest::Run
         { env: Vec<String> }      ← #[serde(default)] → empty if omitted
         JSON over Unix socket
                  │
                  ▼
         daemonbox/server.rs
         dispatch() → handle_run(…, env, …)
                  │
                  ▼
         daemonbox/handler.rs
         container_env = [
           "PATH=/usr/local/sbin:…",  ← always prepended
           "TERM=xterm",              ← always prepended
           …user env vars…            ← appended from request
         ]
         ContainerSpawnConfig { env: container_env }
                  │
                  ▼
         linuxbox/container/process.rs
         child_init()
         envp: Vec<CString> built from config.env
                  │
                  ▼
         execve(cmd, argv, envp)    ← explicit envp, no parent inheritance
```

## Why execve not execvp

`execvp` inherits the **parent process** environment (the daemon's env), which
leaks host secrets into containers. `execve` takes an explicit `envp` argument,
giving the container exactly the vars listed in `ContainerSpawnConfig::env` —
no more, no less.

Defaults (`PATH`, `TERM`) are prepended in `handler.rs` before the user vars so
they can be overridden by the caller (e.g. `env: ["PATH=/custom/bin"]` replaces
the default).
