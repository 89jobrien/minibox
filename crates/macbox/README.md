# macbox

`macbox` supplies VM-backed platform support used by `miniboxd`, primarily for macOS. It owns the
krun implementation, Colima dependency composition, macOS paths/preflight helpers, and the
feature-gated Apple Virtualization.framework (VZ) experiment.

The crate is internal to the workspace (`publish = false`). Adapter selection itself is owned by
`miniboxd::adapter_registry`.

## Architecture role

`macbox` is an outer infrastructure ring. It depends on the `minibox` application layer and
implements domain ports using VM or external CLI boundaries. It must not define new canonical
protocol or domain types.

Current composition is split as follows:

- `miniboxd` builds the Colima suite through `build_colima_handler_dependencies`;
- `miniboxd` imports the krun adapters through the `smolbox` compatibility facade; and
- the VZ startup path calls `macbox::start()` because VZ needs the real macOS main thread for GCD
  callbacks.

## Backends

### krun

The `krun` module exports:

- `KrunRegistry` for image pull/cache integration;
- `KrunRuntime` and `SmolvmProcess` for VM process lifecycle and output;
- `KrunFilesystem` for the filesystem port; and
- `KrunLimiter` for resource configuration.

The process implementation invokes the external `smolvm` command as the libkrun-facing runner.
Krun is the automatic macOS fallback when the preferred smolvm suite is unavailable.

### Colima

The Colima adapter types are implemented by `minibox::adapters`; `macbox` owns their daemon
dependency composition. Operations delegate through `colima ssh --` to tools such as `nerdctl`
inside the Lima VM.

Colima currently wires image pull/load, run/stop/list, push, commit, and build. The shared exec
runtime is unset, so exec into a running container and normal historical logs remain limited.

```bash
colima start
MINIBOX_ADAPTER=colima ./target/release/miniboxd
```

### VZ feature

The optional `vz` feature enables Objective-C bindings for Apple's Virtualization.framework,
vsock proxying, VM configuration, and VZ adapter types:

```bash
cargo build -p miniboxd --features vz
MINIBOX_ADAPTER=vz ./target/debug/miniboxd
```

VZ is currently blocked during Linux VM boot by `VZLinuxBootLoader` failures on the tested macOS
environment. It is not a production backend. It also requires pre-provisioned kernel, initramfs,
and rootfs assets under the VM directory returned by `vz::vm::default_vm_dir()`.

## Public API

| API                                       | Purpose                                                             |
| ----------------------------------------- | ------------------------------------------------------------------- |
| `build_colima_handler_dependencies`       | Compose the Colima handler dependency suite                         |
| `krun::*`                                 | Krun runtime, registry, filesystem, limiter, and process types      |
| `paths::{data_dir, run_dir, socket_path}` | macOS defaults                                                      |
| `preflight`                               | Detect and start Colima through an injectable executor              |
| `start`                                   | Legacy/full macOS daemon entry used by the VZ-specific startup path |
| `vz::*`                                   | Feature-gated VZ VM, adapters, proxy, and vsock support             |

## Platform and feature constraints

- The crate can compile on Unix hosts for conformance and shared VM code, but VZ is macOS-only in
  practice.
- `vz` is off by default and pulls in Objective-C/Virtualization.framework dependencies.
- Colima requires the `colima` CLI and a running VM.
- Krun requires the external smolvm/libkrun execution path.
- VM suites do not currently expose native bind mounts, bridge networking, pause/resume, or
  Linux-native exec parity.

## Development and testing

Krun has adapter and port conformance tests. VZ has smoke/isolation tests, but successful boot is
environment-dependent and currently blocked.

```bash
cargo check -p macbox
cargo clippy -p macbox --all-targets -- -D warnings
cargo nextest run -p macbox
cargo xtask test krun-conformance
```

Compile the optional surface separately:

```bash
cargo check -p macbox --features vz
```
