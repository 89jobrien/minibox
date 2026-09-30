# minibox

`minibox` is the runtime application and infrastructure layer for the minibox container system.
It combines concrete adapters with daemon request handling, persistent state, policy enforcement,
and Linux-native container primitives.

This package is internal to the workspace (`publish = false`). Applications normally use the
`mbx` CLI, the `miniboxd` daemon, or the lower-level published `minibox-core` and
`minibox-domain` crates.

## Architecture role

Domain ports are owned by `minibox-domain`; shared protocol and image services are owned by
`minibox-core`. This crate implements and orchestrates those contracts:

```text
minibox-domain ports
        ^
minibox-core protocol and shared services
        ^
minibox adapters, handlers, server, state, and Linux primitives
        ^
miniboxd composition root
```

`minibox::domain`, `minibox::protocol`, `minibox::image`, and `minibox::preflight` are
compatibility re-exports. They also preserve paths required by `minibox-macros` expansions.

## Main modules

| Module              | Purpose                                                                 |
| ------------------- | ----------------------------------------------------------------------- |
| `adapters`          | Native, GKE, Colima, smolvm, Docker Desktop, and platform stub adapters |
| `daemon::handler`   | Protocol request routing and injected `HandlerDependencies`             |
| `daemon::server`    | Framed socket handling, streaming responses, and peer credentials       |
| `daemon::state`     | Container records, persistence, lookup, and startup reconciliation      |
| `daemon::telemetry` | No-op/Prometheus metrics and optional OTLP tracing                      |
| `container`         | Linux namespaces, cgroups v2, mounts, `pivot_root`, and process spawn   |
| `container_state`   | State-handle contracts used by adapters                                 |
| `fs_util`           | Cross-platform recursive copy and container `/dev` layout helpers       |
| `nesting`           | Nesting-depth validation and nested-overlay capability probing          |
| `resource_limits`   | Cross-platform validation of cgroup v2 resource-limit bounds            |
| `error`             | Diagnostic filesystem, cgroup, namespace, and process error types       |
| `testing`           | Mocks, fixtures, backends, reports, and helpers behind `test-utils`     |

The Linux-only `container` module is compiled only for `target_os = "linux"`. The daemon and
adapter application layers remain available on Unix so VM-backed suites can run on macOS.

## Adapter implementations

| Adapter family | Key implementations                                                    | Status               |
| -------------- | ---------------------------------------------------------------------- | -------------------- |
| Native Linux   | `OverlayFilesystem`, `CgroupV2Limiter`, `LinuxNamespaceRuntime`        | Production           |
| Native exec    | `NativeExecRuntime`                                                    | Linux only           |
| GKE            | `CopyFilesystem`, `NoopLimiter`, `ProotRuntime`                        | Linux, unprivileged  |
| Colima         | `ColimaRegistry`, `ColimaFilesystem`, `ColimaLimiter`, `ColimaRuntime` | Experimental         |
| smolvm         | `SmolVmRegistry`, `SmolVmFilesystem`, `SmolVmLimiter`, `SmolVmRuntime` | Experimental         |
| Shared network | `NoopNetwork`, `HostNetwork`; bridge under Linux                       | Capability-dependent |
| Stubs          | Docker Desktop, HCS, WSL2, Virtualization.framework facade types       | Not daemon-wired     |

The actual suite selection and dependency composition live in `miniboxd`, not in this crate.

## Handler dependencies and policy

`HandlerDependencies` groups infrastructure by concern:

- `ImageDeps`: registry routing, image loading, garbage collection, and local storage;
- `LifecycleDeps`: filesystem, resource limiter, runtime, networking, and directories;
- `ExecDeps`: optional exec runtime and PTY session registry;
- `BuildDeps`: optional push, commit, and build adapters;
- `EventDeps`: event sink/source and metrics recorder;
- deny-by-default `ContainerPolicy`, optional `ExecutionPolicy`, and checkpoint port.

Bind mounts and privileged runs are denied unless enabled by operator configuration. The daemon
composition root reads `MINIBOX_ALLOW_BIND_MOUNTS` and `MINIBOX_ALLOW_PRIVILEGED`.

## Features

| Feature      | Default | Effect                                                                     |
| ------------ | ------- | -------------------------------------------------------------------------- |
| `test-utils` | Yes     | Compile mocks, fixtures, conformance, and test helpers                     |
| `registry`   | Yes     | Enable reqwest-based registry, GHCR, push, and smolvm registry code        |
| `metrics`    | No      | Enable Prometheus recorder and HTTP metrics endpoint support               |
| `otel`       | No      | Enable OTLP/gRPC trace export support                                      |
| `cni`        | No      | Use `minibox-cni` for native bridge networking when selected by `miniboxd` |

For a smaller local-only build, disable default features and opt into only the required services.

## Library usage

```rust
use minibox::domain::NetworkMode;
use minibox::protocol::DaemonRequest;

let request = DaemonRequest::List;
assert_eq!(request.type_tag(), "List");
let _network = NetworkMode::None;
```

Most adapter construction requires storage paths and shared state; follow the builders in
`crates/miniboxd/src/main.rs` rather than constructing a partial suite ad hoc.

## Platform constraints

- Native isolation requires Linux, root, cgroups v2, overlayfs, and appropriate namespace support.
- GKE uses proot and a copy filesystem; it does not provide native cgroups or namespaces.
- macOS execution is VM-backed through suites composed by `miniboxd` and `macbox`.
- Native exec is Linux-only. VM-backed adapters currently leave `ExecDeps::exec_runtime` unset.
- Checkpoint protocol handlers are present, but production suites currently wire
  `NoopVmCheckpoint`.

## Development and testing

The crate has extensive adapter conformance, handler, state, security, property, concurrency, and
Linux isolation tests.

```bash
cargo check -p minibox --all-features
cargo clippy -p minibox --all-targets --all-features -- -D warnings
cargo nextest run -p minibox --all-features
cargo xtask test unit
cargo xtask test property
```

The complete package command includes the crate's integration-test targets. Tests that need real
Linux kernel isolation skip when capabilities are absent; run the privileged suites separately on
a capable Linux host:

```bash
cargo xtask test integration
cargo xtask test sandbox
```

After adapter or dependency-wiring changes, also run `cargo xtask architecture` and the relevant
conformance suite.
