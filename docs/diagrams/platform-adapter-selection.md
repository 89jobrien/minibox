# Platform Adapter Selection

## Description

At startup, `miniboxd` detects the host platform and delegates to the appropriate
platform crate. Within each platform crate, `preflight()` checks which backends
are available and selects one — either via the `MINIBOX_ADAPTER` env var (explicit)
or by capability probing (auto). A fatal error is reported before the socket is
bound if no backend is available.

## ASCII

```
miniboxd starts
      │
      ├─── Linux ──────────────────────────────────────────────┐
      │      │                                                 │
      │    MINIBOX_ADAPTER?                                    │
      │      ├── native (default) → namespaces + cgroups v2    │
      │      ├── gke              → proot + copy FS            │
      │      └── colima           → Colima/limactl delegate    │
      │                                                        │
      ├─── macOS ───────────────────────────────────────────── ┤
      │      │                                                 │
      │    macbox::preflight()                                 │
      │      ├── MINIBOX_ADAPTER=vz  OR  VF available  ───────►│ Virtualization.framework
      │      ├── MINIBOX_ADAPTER=colima  OR  Colima running ──►│ Colima delegate
      │      └── neither ──────────────────────────────────── ►│ FATAL: no backend
      │                                                        │
      └─── Windows ─────────────────────────────────────────── ┘
             │
           winbox::preflight()
             ├── MINIBOX_ADAPTER=hcs   OR  HCS available  ───► HCS (Windows Containers)
             ├── MINIBOX_ADAPTER=wsl2  OR  WSL2 available ───► WSL2 delegate
             └── neither ─────────────────────────────────── ► FATAL: no backend
```

## Mermaid

```mermaid
flowchart TD
    start([miniboxd starts]) --> detect{Host platform?}

    detect -->|Linux| linux_env{MINIBOX_ADAPTER?}
    detect -->|macOS| mac_pre[macbox::preflight]
    detect -->|Windows| win_pre[winbox::preflight]

    linux_env -->|native / unset| native["Native\nnamespaces + overlay + cgroups v2"]
    linux_env -->|gke| gke["GKE\nproot + copy FS + no-op limiter"]
    linux_env -->|colima| colima_linux["Colima\nlimactl delegate"]

    mac_pre --> mac_env{MINIBOX_ADAPTER set?}
    mac_env -->|vz| vf["Virtualization.framework\nshared Linux VM"]
    mac_env -->|colima| colima_mac["Colima\nlimactl delegate"]
    mac_env -->|unset| mac_probe{VF available?}
    mac_probe -->|yes| vf
    mac_probe -->|no| mac_col{Colima running?}
    mac_col -->|yes| colima_mac
    mac_col -->|no| mac_fail[/"FATAL: no backend — install Colima or upgrade macOS"/]

    win_pre --> win_env{MINIBOX_ADAPTER set?}
    win_env -->|hcs| hcs["HCS\nWindows Containers"]
    win_env -->|wsl2| wsl2["WSL2\nLinux OCI in WSL2 distro"]
    win_env -->|unset| win_probe{HCS available?}
    win_probe -->|yes| hcs
    win_probe -->|no| win_wsl{WSL2 available?}
    win_wsl -->|yes| wsl2
    win_wsl -->|no| win_fail[/"FATAL: no backend — enable Windows Containers or install WSL2"/]

    native --> bind[Bind socket / pipe\nStart daemonbox server]
    gke --> bind
    colima_linux --> bind
    vf --> bind
    colima_mac --> bind
    hcs --> bind
    wsl2 --> bind

    style mac_fail fill:#f8d7da
    style win_fail fill:#f8d7da
    style bind fill:#d4edda
```
