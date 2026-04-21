# Conformance Suite

The conformance suite validates that every backend adapter (commit, build, push) honours the
contracts defined by its domain trait. Tests live in `crates/minibox/tests/conformance_*.rs` and
use the fixture infrastructure from `minibox_core::adapters::conformance`.

## Running the suite

### Any platform (macOS, Linux — no daemon required)

```bash
cargo xtask test-conformance
```

This runs three test binaries in sequence then emits artifact reports:

```
artifacts/conformance/report.md   — human-readable Markdown table
artifacts/conformance/report.json — machine-readable JSON
```

### Individual test files

```bash
cargo test --release -p minibox --test conformance_commit
cargo test --release -p minibox --test conformance_build
cargo test --release -p minibox --test conformance_push
```

### Emitting reports only (after tests pass)

```bash
cargo test --release -p minibox --test conformance_report -- --nocapture
```

Override the output directory:

```bash
CONFORMANCE_ARTIFACT_DIR=/tmp/my-reports \
  cargo test --release -p minibox --test conformance_report -- --nocapture
```

## Running with Colima (macOS)

The conformance suite targets the minibox-native backend by default and does not require a
running daemon. To validate the Colima adapter:

1. Start Colima: `colima start`
2. Set `MINIBOX_ADAPTER=colima`
3. Run `cargo xtask test-conformance`

> **Note:** Colima adapter conformance requires the `macbox` crate to be compiled with its
> default feature set and Colima/nerdctl available on PATH. Linux-native tests should always
> run on a real Linux host or a Linux CI runner — do not substitute Colima for linux-native
> coverage in CI.

## Running push tests with a local registry

Push conformance is split into two tiers:

| Tier | Condition                       | What runs                                       |
| ---- | ------------------------------- | ----------------------------------------------- |
| 1    | always                          | Descriptor wiring, capability checks            |
| 2    | `CONFORMANCE_PUSH_REGISTRY` set | Full push roundtrip against a real OCI registry |

To activate tier 2:

```bash
# Start a local OCI registry
docker run -d -p 5000:5000 --name registry registry:2

# Run the suite with push registry
CONFORMANCE_PUSH_REGISTRY=localhost:5000 cargo xtask test-conformance
```

The `LocalPushTargetFixture` always targets `localhost:5000`. The env var must match.

## Capability matrix

| Backend                 | Commit | BuildFromContext | PushToRegistry |
| ----------------------- | ------ | ---------------- | -------------- |
| `minibox-native-commit` | yes    | no               | no             |
| `minibox-native-build`  | no     | yes              | no             |
| `minibox-native-push`   | no     | no               | yes            |
| `linux-native` (future) | yes    | yes              | yes            |
| `colima` (future)       | yes    | yes              | yes            |

Backends that do not declare a capability have their corresponding conformance tests skipped
(not failed). The skip is recorded in the report matrix.

## Linux-native coverage

The linux-native adapter requires:

- A real Linux host or CI runner (kernel 5.0+, cgroups v2, overlay FS)
- Root privileges for namespace/cgroup operations

Do **not** run linux-native conformance tests on macOS with Colima — the adapter paths diverge
and Colima substitution produces false coverage. The CI `next`/`stable` branch gates run the
full suite on a dedicated Linux runner.

## Artifacts in CI

CI writes conformance artifacts to `artifacts/conformance/` as part of the `test-unit` gate on
`next` and `stable` branches. The JSON report is consumed by the bench dashboard
(`just dash`) under the CI tab.
