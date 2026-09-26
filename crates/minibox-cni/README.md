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

| API                                          | Purpose                                                             |
| -------------------------------------------- | ------------------------------------------------------------------- |
| `NetworkConfigList::from_file`               | Parse a CNI `.conflist` JSON file                                   |
| `NetworkConfigList::checked_spec_version`    | Validate the file's `cniVersion` against the crate constant         |
| `NetworkConfigList::add`                     | Run `ADD` in chain order and thread `prevResult`                    |
| `NetworkConfigList::del`                     | Run best-effort `DEL` in reverse chain order                        |
| `PluginConfig`                               | Preserve a plugin's type and plugin-defined JSON fields             |
| `CniNetworkProvider`                         | Implement the minibox networking port through a CNI chain           |
| `CniResult` and related types                | Deserialize CNI result and error payloads                           |
| `CniError`                                   | Distinguish config, lookup, process, and structured plugin failures |
| `PluginRequest::decode` / `plugin::dispatch` | Plugin-side exec protocol, validated against the spec version       |
| `version_info` / `validate_spec_version`     | The one place the CNI spec version is written down                  |
| `CniRollout` / `CniRollout::preflight`       | The validated rollout contract and its operator-facing preflight    |

If an `ADD` fails partway through a chain, successful earlier plugins receive rollback `DEL`
calls in reverse order. Teardown continues after individual `DEL` failures and logs them. Both
paths reject an unsupported `cniVersion` before spawning anything.

## The CNI spec version has exactly one source of truth

`version::CNI_SPEC_VERSION` is the only place the literal `"1.0.0"` appears. Four paths read it
rather than repeating it:

- `CNI_COMMAND=VERSION` replies with `version_info()`, derived from the constant.
- Plugin-side `ADD`/`DEL`/`CHECK` validate the config's `cniVersion` against
  `validate_spec_version` before a backend is consulted.
- `NetworkConfigList::add`/`del` validate the `.conflist`'s `cniVersion` before spawning a plugin.
- `CniRollout::preflight` checks the `.conflist` and every installed plugin binary's advertised
  version against the same constant.

A version therefore cannot be accepted by one path and rejected by another.

## Packaged binaries

| Binary               | Role                                                                            |
| -------------------- | ------------------------------------------------------------------------------- |
| `minibox-cni-bridge` | The CNI plugin. Install on `CNI_PATH`, reference as `"type": "minibox-bridge"`. |
| `minibox-cni-verify` | Rollout preflight. Prints a JSON report, or the error plus an install hint.     |

Both are built with `cargo build -p minibox-cni --bins`.

`minibox-cni-bridge` implements the full exec protocol: it decodes `CNI_COMMAND` plus the
`CNI_CONTAINERID`/`CNI_NETNS`/`CNI_IFNAME`/`CNI_PATH`/`CNI_ARGS` environment, parses the `CNI_ARGS`
`K=V;K2` form, validates the spec version, and emits either a result object on stdout with exit 0
or a spec-shaped error object on stdout with exit 1.

## Architecture

`version`, `plugin`, and `rollout` are the domain layer: pure functions with no filesystem,
process, or kernel access. `adapters` implements the three ports they declare:

| Port                          | Adapter                                      | Tests use                  |
| ----------------------------- | -------------------------------------------- | -------------------------- |
| `plugin::PluginEnvironment`   | `adapters::ProcessEnvironment`               | an in-memory `BTreeMap`    |
| `plugin::NamespaceAttachment` | `adapters::UnimplementedNamespaceAttachment` | an in-memory recorder      |
| `rollout::PluginProbe`        | `adapters::ExecPluginProbe`                  | an in-memory install table |

No test needs root, a real network namespace, or a real CNI plugin on disk.

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

`CniRollout` encodes these expectations — plus the `cni` daemon feature, the `bridge` network
mode, and the `minibox-bridge` binary name — as a validated struct with defaults matching
`miniboxd`. Drift between the struct and the daemon is a test failure.

A minimal chain has the standard CNI shape:

```json
{
  "cniVersion": "1.0.0",
  "name": "minibox0",
  "plugins": [
    { "type": "minibox-bridge", "bridge": "minibox0" },
    { "type": "portmap", "capabilities": { "portMappings": true } }
  ]
}
```

Build and start the daemon with CNI-backed bridge networking:

```bash
cargo build -p minibox-cni --bins
sudo install -m 0755 target/debug/minibox-cni-bridge /opt/cni/bin/minibox-bridge
sudo install -m 0644 10-minibox.conflist /etc/cni/net.d/10-minibox.conflist
cargo build -p miniboxd --features cni
MINIBOX_CNI_PATH=/opt/cni/bin ./target/debug/minibox-cni-verify
sudo MINIBOX_ADAPTER=native MINIBOX_NETWORK_MODE=bridge ./target/debug/miniboxd
```

## Library example

```rust
use minibox_cni::{CniNetworkProvider, CniRollout};
use std::path::PathBuf;

let provider = CniNetworkProvider::new(
    vec![PathBuf::from("/opt/cni/bin")],
    PathBuf::from("/etc/cni/net.d"),
);
assert_eq!(provider.config_dir, PathBuf::from("/etc/cni/net.d"));

let rollout = CniRollout::defaults();
assert_eq!(rollout.conflist_path(), PathBuf::from("/etc/cni/net.d/10-minibox.conflist"));
```

## Constraints and implementation status

- The production integration is Linux-specific because it attaches `/proc/<pid>/ns/net`.
- Plugin names must be a single normal path component; traversal and embedded paths are rejected.
- Plugin config is written to stdin and results are read as JSON from stdout.
- `CniNetworkProvider::stats` is not implemented and returns an error.
- **Namespace attachment is not implemented.** `adapters::UnimplementedNamespaceAttachment`
  reports a structured CNI error from `ADD` and `CHECK` rather than claiming networking that
  was never created; `DEL` is a no-op, which is spec-correct given `ADD` always fails. A netlink
  `NamespaceAttachment` implementation is the remaining piece. Everything around it — the exec
  protocol, spec-version validation, error mapping, and the rollout preflight — is complete
  and tested.
- The adapter tracks namespace paths in memory, so cleanup requires a successful prior `attach` in
  the same provider process.

## Development and testing

Tests use temporary executable fixture plugins and in-memory port doubles to cover spec-version
agreement, spec-version acceptance and rejection, `CNI_ARGS` parsing, command dispatch, error-code
mapping, config parsing with defaults and overrides, and the rollout preflight. The integration
tests in `tests/plugin_binaries.rs` run the real binaries.

```bash
cargo check -p minibox-cni
cargo clippy -p minibox-cni --all-targets -- -D warnings
cargo nextest run -p minibox-cni
cargo xtask test unit
```
