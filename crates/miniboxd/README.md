# miniboxd

`miniboxd` is the minibox composition root and long-running daemon. It selects an adapter suite,
constructs `HandlerDependencies`, restores daemon state, secures the local transport, and serves
the newline-delimited JSON protocol defined by `minibox-core`.

The package also exposes a small library for adapter discovery, layered configuration, the Unix
listener, and backward-compatible daemon module paths used by integration tests.

## Startup and architecture

On Unix, startup proceeds through these boundaries:

1. parse the two supported command-line options;
2. load system/user TOML and environment configuration;
3. resolve and validate the adapter suite;
4. create data/runtime directories and restore persistent state;
5. compose image, lifecycle, exec, build, event, policy, and checkpoint dependencies;
6. bind and secure the Unix socket; and
7. run the shared `minibox::daemon::server` accept loop until SIGINT or SIGTERM.

Windows dispatches to `winbox::start()`, which is currently a stub and returns an error.

## Command-line options

`miniboxd` uses a small manual parser rather than Clap. It does not currently provide generated
`--help` or `--version` output.

| Option                                  | Meaning                                                      |
| --------------------------------------- | ------------------------------------------------------------ |
| `--adapter <name>` / `--adapter=<name>` | Override adapter selection                                   |
| `--restart`                             | Send SIGTERM to existing `miniboxd` processes before startup |

Unknown options are currently ignored. Adapter precedence is:

```text
--adapter > MINIBOX_ADAPTER > config.toml adapter > automatic selection
```

## Adapter selection

| Value    | Availability              | Current role                                           |
| -------- | ------------------------- | ------------------------------------------------------ |
| `native` | Linux only, root required | Namespaces, overlayfs, cgroups v2, native exec         |
| `gke`    | Linux only                | Unprivileged proot and copy-based filesystem           |
| `colima` | Unix                      | Delegates through Colima/Lima and nerdctl              |
| `smolvm` | Unix                      | Preferred automatic default when `smolvm` is on `PATH` |
| `krun`   | Unix runtime path         | Automatic fallback on macOS                            |
| `vz`     | macOS plus `vz` feature   | Separate, currently blocked VZ startup path            |

If `MINIBOX_ADAPTER` is unset, the daemon probes smolvm first. It falls back to `native` on Linux
or `krun` on macOS. An explicit but unavailable adapter is a hard error and never falls back.

```bash
cargo build -p miniboxd
sudo ./target/debug/miniboxd --adapter native
RUST_LOG=debug ./target/debug/miniboxd --adapter smolvm
```

## Configuration files

`DaemonConfig::load()` overlays these sources in order:

1. `/etc/minibox/config.toml`;
2. `$HOME/.config/minibox/config.toml`; and
3. supported `MINIBOX_*` environment variables.

The current daemon startup consumes the TOML `adapter` and policy fields. Runtime paths are
resolved from the path environment variables shown below; the public config struct also contains
path fields that are not yet applied by `run_daemon`.

```toml
adapter = "native"
log_level = "info"

[policy]
allow_privileged = false
allow_bind_mounts = false
max_image_size_mb = 2048
```

## Environment

| Variable                    | Purpose                                                                   |
| --------------------------- | ------------------------------------------------------------------------- |
| `MINIBOX_ADAPTER`           | Select an adapter suite                                                   |
| `MINIBOX_DATA_DIR`          | Image, container, lease, and state data root                              |
| `MINIBOX_RUN_DIR`           | Runtime directory; contains the default socket and container runtime data |
| `MINIBOX_SOCKET_PATH`       | Override the Unix socket path                                             |
| `MINIBOX_SOCKET_MODE`       | Override socket permissions as octal text                                 |
| `MINIBOX_SOCKET_GROUP`      | Assign the socket to a local group                                        |
| `MINIBOX_CGROUP_ROOT`       | Override the native cgroup root                                           |
| `MINIBOX_NETWORK_MODE`      | `none`, `bridge`, `host`, or feature-gated `tailnet`                      |
| `MINIBOX_ALLOW_BIND_MOUNTS` | Permit bind mounts in run requests                                        |
| `MINIBOX_ALLOW_PRIVILEGED`  | Permit privileged run requests                                            |
| `MINIBOX_METRICS_ADDR`      | Prometheus listener, default `127.0.0.1:9090`                             |
| `MINIBOX_OTLP_ENDPOINT`     | OTLP/gRPC trace endpoint                                                  |
| `RUST_LOG`                  | Tracing filter                                                            |

With the `cni` feature and bridge mode, `MINIBOX_CNI_PATH` and
`MINIBOX_CNI_CONFIG_DIR` configure the CNI adapter.

## Features

| Feature   | Default | Effect                                                             |
| --------- | ------- | ------------------------------------------------------------------ |
| `metrics` | Yes     | Prometheus recorder and metrics HTTP endpoint                      |
| `otel`    | Yes     | OTLP tracing export                                                |
| `cni`     | No      | CNI-backed native bridge networking                                |
| `vz`      | No      | macOS Virtualization.framework path                                |
| `tailnet` | No      | Reserved cfg gate; external tailbox dependency is not present here |

## Platform and security constraints

- Native mode requires UID 0, Linux namespaces, cgroups v2, and overlayfs.
- Native Unix socket requests use peer credentials and require root for the native suite.
- Socket permissions default to `0600`; group/mode overrides are operator-controlled.
- Container state is persisted and reconciled at startup, but running processes are not reattached
  as managed child processes.
- Production checkpoint wiring currently uses `NoopVmCheckpoint`.
- VZ requires a separate main-thread/GCD startup path and remains nonfunctional at VM boot.

## Development and testing

Tests cover config/adapter selection, socket credentials, state races and reconciliation, framed
protocol behavior, daemon/CLI flows, smolvm smoke cases, property tests, and Linux/root suites.

```bash
cargo check -p miniboxd
cargo clippy -p miniboxd --all-targets -- -D warnings
cargo nextest run -p miniboxd
cargo xtask test e2e
```

The commands above exercise the default features only. Do not use `--all-features` for Linux
builds. The `cni` feature is currently non-buildable on Linux: the manifest forwards it to
`minibox/cni`, but Linux-only daemon code directly references `minibox_cni` without declaring a
direct `minibox-cni` dependency. A check on another platform can skip that code through
`cfg(target_os = "linux")` and therefore does not validate the CNI path. Until the dependency
wiring is fixed, do not use or recommend `cargo check -p miniboxd --features cni` for Linux.

The `tailnet` gate references the external `tailbox` crate, which is not a dependency of this
workspace. That feature is also not buildable on Linux until the external plugin is integrated.

Privileged Linux validation is separate from the complete default-feature package and protocol
e2e tests:

```bash
cargo xtask test integration
cargo xtask test system-suite
cargo xtask test sandbox
```
