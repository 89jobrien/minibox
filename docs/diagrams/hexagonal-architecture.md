# Hexagonal Architecture

## Description

Minibox uses hexagonal (ports and adapters) architecture. The domain ports
(traits) in `minibox/src/domain.rs` define the interfaces. The application
core in `daemonbox` depends only on those traits — it never imports a concrete
adapter. Composition roots (`miniboxd/main.rs` for Linux, `macbox::start()` for
macOS, `winbox::start()` for Windows) are the only place where concrete adapters
are wired to the application core.

This means the daemon logic (handler, state, server) is fully testable with mock
adapters and has zero platform-specific code.

## ASCII

```
╔═══════════════════════════════════════════════════════════════════════╗
║               COMPOSITION ROOTS  (driving side)                       ║
║  miniboxd/main.rs        macbox::start()      winbox::start()         ║
╠═══════════════════════════════════════════════════════════════════════╣
║                     APPLICATION CORE                                  ║
║                         daemonbox                                     ║
║               handler.rs   state.rs   server.rs                       ║
║          (depends only on domain port traits — no cfg blocks)         ║
╠═══════════════════════════════════════════════════════════════════════╣
║                     DOMAIN PORTS                                      ║
║               minibox/src/domain.rs                                   ║
║    ContainerRuntime   FilesystemProvider                              ║
║    ImageRegistry      ResourceLimiter      NetworkProvider            ║
╠═══════════════════════════════════════════════════════════════════════╣
║                     DRIVEN ADAPTERS                                   ║
║              minibox/src/adapters/                                    ║
║                                                                       ║
║  Linux:    LinuxNamespaceRuntime  OverlayFilesystem  CgroupV2Limiter  ║
║            DockerHubRegistry      GhcrRegistry       NativeExecRuntime║
║            ProotRuntime           CopyFilesystem      NoopLimiter     ║
║            Wsl2Runtime            Wsl2Filesystem      Wsl2Limiter     ║
║            HcsRuntime             HcsFilesystem       HcsLimiter      ║
║            HcsRegistry                                                ║
║                                                                       ║
║  macOS:    ColimaRuntime          ColimaFilesystem    ColimaLimiter   ║
║            ColimaRegistry                                             ║
║            VzRuntime (macbox)     VzFilesystem        VzLimiter       ║
║            VzRegistry (macbox)                                        ║
║            SmolVmRuntime          SmolVmFilesystem    SmolVmLimiter   ║
║            SmolVmRegistry  [krun path — macbox/src/krun/]            ║
╚═══════════════════════════════════════════════════════════════════════╝
```

## Mermaid

```mermaid
graph TB
    subgraph roots["Composition Roots (driving side)"]
        linux_root["miniboxd/main.rs\n(Linux)"]
        mac_root["macbox::start()\n(macOS)"]
        win_root["winbox::start()\n(Windows)"]
    end

    subgraph core["Application Core — daemonbox"]
        handler["handler.rs"]
        state["state.rs"]
        server["server.rs"]
    end

    subgraph ports["Domain Ports — minibox/domain.rs"]
        runtime_trait["ContainerRuntime"]
        fs_trait["FilesystemProvider"]
        registry_trait["ImageRegistry"]
        limiter_trait["ResourceLimiter"]
    end

    subgraph adapters_linux["Linux Adapters"]
        ln_rt["LinuxNamespaceRuntime"]
        ln_fs["OverlayFilesystem"]
        ln_lim["CgroupV2Limiter"]
        ln_reg["DockerHubRegistry"]
    end

    subgraph adapters_mac["macOS Adapters"]
        col_rt["ColimaRuntime"]
        col_fs["ColimaFilesystem"]
        vz_rt["VzRuntime (macbox)"]
        vz_fs["VzFilesystem (macbox)"]
        smolvm_rt["SmolVmRuntime (krun)"]
        smolvm_fs["SmolVmFilesystem (krun)"]
    end

    subgraph adapters_win["Windows Adapters"]
        hcs_rt["HcsRuntime"]
        hcs_fs["HcsFilesystem"]
        hcs_lim["HcsLimiter"]
        wsl_rt["Wsl2Runtime"]
        wsl_fs["Wsl2Filesystem"]
        wsl_lim["Wsl2Limiter"]
    end

    linux_root -->|"wires"| core
    mac_root -->|"wires"| core
    win_root -->|"wires"| core

    core -->|"depends on"| ports

    ports -.->|"implemented by"| adapters_linux
    ports -.->|"implemented by"| adapters_mac
    ports -.->|"implemented by"| adapters_win

    linux_root -.->|"selects"| adapters_linux
    mac_root -.->|"selects"| adapters_mac
    win_root -.->|"selects"| adapters_win
```
