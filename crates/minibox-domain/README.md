# minibox-domain

`minibox-domain` is the innermost dependency ring of the minibox runtime. It owns the domain
values, policies, lifecycle events, and hexagonal ports shared by the daemon and adapter crates.
It deliberately contains no HTTP client, socket transport, process runner, tracing subscriber,
or concrete filesystem/network adapter.

## Architecture role

The intended dependency direction is:

```text
minibox-domain -> minibox-core -> minibox -> platform crates -> miniboxd
```

`minibox-core::domain` and `minibox::domain` are compatibility re-exports of these same types.
New domain contracts should be defined here rather than duplicated in an outer crate.

## Public API

The crate root re-exports the public contents of its modules. The main API groups are:

| Area                           | Key types and ports                                                              |
| ------------------------------ | -------------------------------------------------------------------------------- |
| Runtime                        | `ContainerRuntime`, `ContainerSpawnConfig`, `RuntimeCapabilities`, `SpawnResult` |
| Resources                      | `ResourceLimiter`, `ResourceConfig`                                              |
| Filesystems                    | `RootfsSetup`, `ChildInit`, `FilesystemProvider`, `RootfsLayout`, `BindMount`    |
| Images                         | `ImageRegistry`, `RegistryRouter`, `ImageLoader`, `ImagePusher`, `ImageBuilder`  |
| Commit                         | `ContainerCommitter`, `CommitConfig`                                             |
| Networking                     | `NetworkProvider`, `NetworkConfig`, `NetworkMode`, `PortMapping`                 |
| Exec and PTY                   | `ExecRuntime`, `ExecSession`, `ExecOutput`, `PtyAllocator`                       |
| Events and metrics             | `ContainerEvent`, `EventSink`, `MetricsRecorder`                                 |
| Identity and state             | `ContainerId`, `SessionId`, `ContainerState`, `InternalPath`                     |
| Integrity                      | `ExecutionManifest`, `ExecutionPolicy`, `PolicyDecision`                         |
| Workflows                      | `WorkflowDef`, `WorkflowStep`, `StepRunner`, `StepRunnerRegistry`                |
| Checkpoints                    | `VmCheckpoint`, `SnapshotInfo`, `NoopVmCheckpoint`                               |
| Capabilities (`capability`)    | `BackendCapability`, `BackendCapabilitySet`                                      |
| Progress (`progress`)          | `ProgressSink`, `DynProgressSink`, `ProgressClosed`                              |
| Extension ports (`extensions`) | `TtyProvider`, `ExecProvider`, `LogProvider`, `StateStore`                       |

The `Dyn*` aliases wrap commonly injected ports in `Arc<dyn Trait>`. `AsAny` supports adapter
downcasting in composition and test code.

## Usage

```toml
[dependencies]
minibox-domain = "0.33"
```

Parse and validate domain values before crossing into infrastructure code:

```rust
use minibox_domain::{BindMount, ContainerId, NetworkConfig, NetworkMode};

let id = ContainerId::new("abc123".to_string())?;
let mount = BindMount::parse_volume("/tmp/input:/work/input:ro")?;
let network = NetworkConfig {
    mode: NetworkMode::None,
    ..NetworkConfig::default()
};

assert_eq!(id.as_str(), "abc123");
assert!(mount.read_only);
assert_eq!(network.mode, NetworkMode::None);
# Ok::<(), anyhow::Error>(())
```

Execution manifests hash environment values rather than storing plaintext and derive a stable
workload digest from semantic inputs:

```rust
use minibox_domain::ExecutionManifestEnvVar;

let entry = ExecutionManifestEnvVar::new("TOKEN", "secret-value");
assert_eq!(entry.name, "TOKEN");
assert_ne!(entry.value_digest, "secret-value");
```

## Constraints and features

- Rust 2024; workspace MSRV is Rust 1.85.
- `default` is empty.
- `test-utils` exposes test-oriented domain helpers; it does not add infrastructure adapters.
- Ports may be async through `async-trait`, but the crate does not depend on an async runtime in
  production.
- `NetworkMode::None` is the default. Adapter support for other modes is discovered and enforced
  outside this crate.
- `ExecutionPolicy::default()` imposes no manifest constraints; daemon container policy is a
  separate deny-by-default boundary in the `minibox` crate.

## Development and testing

Most tests are inline unit and property tests because this crate is the pure contract layer.

```bash
cargo check -p minibox-domain
cargo clippy -p minibox-domain --all-targets -- -D warnings
cargo nextest run -p minibox-domain
cargo xtask architecture
```

Changing a public port affects implementations and conformance tests in outer crates. Run the
workspace unit and architecture gates after contract changes:

```bash
cargo xtask test unit
cargo xtask verify
```
