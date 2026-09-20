# minibox-cni

`minibox-cni` implements the Container Network Interface (CNI) plugin exec protocol and ordered
plugin-chain orchestration used by minibox's native Linux adapter. It is an internal workspace
crate (`publish = false`).

## Architecture role

The crate is an infrastructure adapter for `minibox_domain::NetworkProvider` (re-exported through
`minibox-core`). It does not create network namespaces. Callers provide an opaque namespace target;
the production provider records `/proc/<pid>/ns/net` after the container process starts.

`miniboxd` selects `CniNetworkProvider` only when all of these are true:

- the daemon is built with the `cni` feature;
- the native Linux adapter is selected; and
- `MINIBOX_NETWORK_MODE=bridge`.

Without the feature, bridge mode continues to use minibox's built-in `BridgeNetwork` adapter.

## Public API

| API                            | Purpose                                                             |
| ------------------------------ | ------------------------------------------------------------------- |
| `NetworkConfigList::from_file` | Parse a CNI `.conflist` JSON file                                   |
| `NetworkConfigList::add`       | Run `ADD` in chain order and thread `prevResult`                    |
| `NetworkConfigList::del`       | Run best-effort `DEL` in reverse chain order                        |
| `PluginConfig`                 | Preserve a plugin's type and plugin-defined JSON fields             |
| `CniNetworkProvider`           | Implement the minibox networking port through a CNI chain           |
| `CniResult` and related types  | Deserialize CNI result and error payloads                           |
| `CniError`                     | Distinguish config, lookup, process, and structured plugin failures |

If an `ADD` fails partway through a chain, successful earlier plugins receive rollback `DEL`
calls in reverse order. Teardown continues after individual `DEL` failures and logs them.

## Configuration

The production provider reads one fixed configuration file:

```text
$MINIBOX_CNI_CONFIG_DIR/10-minibox.conflist
```

Daemon defaults are:

| Variable                 | Default          | Meaning                               |
| ------------------------ | ---------------- | ------------------------------------- |
| `MINIBOX_CNI_PATH`       | `/opt/cni/bin`   | Platform-separated plugin search path |
| `MINIBOX_CNI_CONFIG_DIR` | `/etc/cni/net.d` | Directory containing the conflist     |

A minimal chain has the standard CNI shape:

```json
{
  "cniVersion": "1.0.0",
  "name": "minibox0",
  "plugins": [
    { "type": "bridge", "bridge": "minibox0" },
    { "type": "portmap", "capabilities": { "portMappings": true } }
  ]
}
```

Build and start the daemon with CNI-backed bridge networking:

```bash
cargo build -p miniboxd --features cni
sudo MINIBOX_ADAPTER=native MINIBOX_NETWORK_MODE=bridge ./target/debug/miniboxd
```

## Library example

```rust
use minibox_cni::CniNetworkProvider;
use std::path::PathBuf;

let provider = CniNetworkProvider::new(
    vec![PathBuf::from("/opt/cni/bin")],
    PathBuf::from("/etc/cni/net.d"),
);
assert_eq!(provider.config_dir, PathBuf::from("/etc/cni/net.d"));
```

## Constraints and implementation status

- The production integration is Linux-specific because it attaches `/proc/<pid>/ns/net`.
- Plugin names must be a single normal path component; traversal and embedded paths are rejected.
- Plugin config is written to stdin and results are read as JSON from stdout.
- `CniNetworkProvider::stats` is not implemented and returns an error.
- Plugin packaging and operational rollout are not part of this crate yet.
- The adapter tracks namespace paths in memory, so cleanup requires a successful prior `attach` in
  the same provider process.

## Development and testing

Tests use temporary executable fixture plugins to cover parsing, `prevResult`, rollback, reverse
teardown, structured errors, process failures, and provider attach/cleanup.

```bash
cargo check -p minibox-cni
cargo clippy -p minibox-cni --all-targets -- -D warnings
cargo nextest run -p minibox-cni
cargo xtask test unit
```
