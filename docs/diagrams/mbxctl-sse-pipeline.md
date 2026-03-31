# mbxctl SSE Streaming Pipeline

Shows how a container job request flows from an HTTP client through mbxctl
into the minibox daemon, and how stdout/stderr streams back as Server-Sent Events.

## Mermaid

```mermaid
sequenceDiagram
    participant Client as HTTP Client<br/>(curl / dagu / browser)
    participant mbxctl as mbxctl<br/>axum server :9999
    participant JobAdapter as JobAdapter
    participant DaemonClient as DaemonClient<br/>(Unix socket)
    participant Daemon as miniboxd<br/>daemonbox handler
    participant Container as Container Process<br/>(execve)

    Client->>mbxctl: POST /jobs<br/>{"image":"alpine","command":["sh"],"env":["FOO=bar"]}
    mbxctl->>JobAdapter: create_and_run(CreateJobRequest)
    JobAdapter->>DaemonClient: call(DaemonRequest::Run { env, ephemeral:true })
    DaemonClient->>Daemon: JSON-over-newline on /run/minibox/miniboxd.sock

    Daemon->>Daemon: pull image if not cached
    Daemon->>Daemon: create overlay mount
    Daemon->>Container: clone(CLONE_NEWPID|CLONE_NEWNS|...) + execve(envp)
    Daemon-->>DaemonClient: ContainerCreated { id }
    DaemonClient-->>JobAdapter: stream open
    JobAdapter-->>mbxctl: (container_id, ResponseStream)
    mbxctl-->>Client: 200 OK {"job_id":"...","container_id":"..."}

    Note over mbxctl,Daemon: drain_container_output runs in background task

    loop stdout / stderr chunks
        Container->>Daemon: write to pipe fd
        Daemon-->>DaemonClient: ContainerOutput { stream: Stdout, data: base64 }
        DaemonClient-->>mbxctl: broadcast LogEvent
    end

    Container->>Daemon: exit(0)
    Daemon-->>DaemonClient: ContainerStopped { exit_code: 0 }
    DaemonClient-->>mbxctl: job status → completed

    Client->>mbxctl: GET /jobs/:id/logs (SSE)
    mbxctl-->>Client: data: {"stream":"stdout","data":"..."}\n\n
    mbxctl-->>Client: data: {"stream":"stdout","data":"..."}\n\n
    mbxctl-->>Client: event: done\n\n
```

## ASCII (fallback)

```text
HTTP Client
    │
    │  POST /jobs {"image":…,"env":["FOO=bar"]}
    ▼
┌─────────────────────────────────┐
│  mbxctl (axum :9999)            │
│  ┌────────────────────────────┐ │
│  │  POST /jobs handler        │ │
│  │  JobAdapter.create_and_run │ │
│  └───────────┬────────────────┘ │
└──────────────┼──────────────────┘
               │  DaemonRequest::Run
               │  (JSON / Unix socket)
               ▼
┌──────────────────────────────────────┐
│  miniboxd (daemonbox)                │
│  server.rs → dispatch()              │
│  handler::handle_run_streaming()     │
│  run_inner_capture()                 │
│    ├── pull image                    │
│    ├── overlay mount                 │
│    └── clone + execve(envp)          │
│             │                        │
│         ContainerOutput chunks       │
│         ContainerStopped { code }    │
└──────────────────────────────────────┘
               │
               │  SSE stream
               ▼
HTTP Client ◄── GET /jobs/:id/logs
```

## Key design points

- `drain_container_output` in `mbxctl/src/server.rs` owns the `ResponseStream`
  returned by `JobAdapter::create_and_run` — no stream is dropped prematurely.
- `ContainerOutput` is the only **non-terminal** `DaemonResponse` variant;
  all others (`ContainerStopped`, `Error`, etc.) end the stream.
- `DefaultBodyLimit(1 MB)` is applied to the axum router to prevent request-body DoS.
- mbxctl binds `localhost:9999` by default (systemd unit) — not internet-facing.
