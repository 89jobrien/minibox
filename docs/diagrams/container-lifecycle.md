# Container Lifecycle

## Description

The state machine for a single container record in `DaemonState`. State transitions
are driven by daemon events (spawn result, process exit, explicit stop/remove).
State is in-memory only — a daemon restart loses all records.

The `waitpid`-based exit detection works on Linux (native + GKE). On macOS
Virtualization.framework and Windows HCS, container processes run inside a VM or
managed runtime — the daemon detects exit via the adapter's `spawn_process` future
resolving, or via polling the platform runtime API. Colima and WSL2 have known
limitations: exit detection may not fire; containers may linger as "Running" until
manually stopped.

## ASCII

```
             RunContainer request
                     │
                     ▼
               ┌──────────┐
               │  Created │ ◄─── record inserted, overlay/rootfs set up
               └────┬─────┘
                    │ spawn_process()
          ┌─────────┴──────────┐
          │ success            │ error
          ▼                    ▼
     ┌─────────┐          ┌────────┐
     │ Running │          │ Failed │
     └────┬────┘          └───┬────┘
          │                   │
          │ exit / SIGTERM     │
          │ / SIGKILL          │
          ▼                   │
     ┌─────────┐              │
     │ Stopped │◄─────────────┘
     └────┬────┘
          │ Remove request
          ▼
      (record deleted,
       overlay unmounted,
       cgroup cleaned up)
```

## Mermaid

```mermaid
stateDiagram-v2
    [*] --> Created : RunContainer request\n(overlay set up, cgroup created)

    Created --> Running : spawn_process() ok\n(PID recorded)
    Created --> Failed : spawn_process() error

    Running --> Stopped : process exited\n(waitpid / adapter event)
    Running --> Stopped : handle_stop()\n(SIGTERM → SIGKILL after 10s)

    Failed --> [*] : handle_remove()

    Stopped --> [*] : handle_remove()\n(overlay unmounted,\ncgroup deleted,\ndirs removed)

    note right of Running
        Exit detection varies by platform:
        Linux native/GKE — waitpid
        macOS VF — adapter future
        macOS Colima — best-effort
        Windows HCS — HCS event
        Windows WSL2 — best-effort
    end note
```
