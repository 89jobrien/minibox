# minibox-bench

`minibox-bench` is the workspace's dedicated Criterion benchmark crate. It is a leaf crate and the
only normal dependency that enables `test-utils` on both `minibox` and `minibox-core`.

The crate is internal (`publish = false`). Its library contains reusable fixtures; performance
targets live under `benches/`.

## Benchmark targets

| Target            | What it measures                                  | Host constraints |
| ----------------- | ------------------------------------------------- | ---------------- |
| `protocol_codec`  | Request/response JSON framing and decoding        | Any platform     |
| `daemon_dispatch` | State reconciliation and mock handler dispatch    | Any platform     |
| `trait_dispatch`  | Direct calls versus dynamic port dispatch         | Any platform     |
| `layer_extract`   | Deterministic OCI layer extraction                | Any platform     |
| `image_pull`      | Pulling synthetic images from a mock OCI registry | Any platform     |
| `linux_rootfs`    | Native rootfs setup and cleanup                   | Linux and root   |
| `linux_cgroup`    | Cgroup v2 create/cleanup                          | Linux and root   |
| `linux_spawn`     | Native process spawn and wait                     | Linux and root   |

Linux-only targets compile as no-ops elsewhere and skip at runtime when the process is not root.

## Running benchmarks

Use the workspace orchestration so results are parsed and stored consistently:

```bash
just bench
just bench-check
just bench-baseline
```

Equivalent lower-level commands include:

```bash
cargo xtask bench
cargo xtask bench --check
cargo xtask bench --save-baseline
cargo xtask bench --skip-bench
```

Run a single Criterion target directly when iterating on a benchmark:

```bash
cargo bench -p minibox-bench --bench protocol_codec -- --noplot
```

`cargo xtask bench` writes parsed runs under `bench/results/`. Tracked baselines are separated by
environment in `bench/baseline.local.json`, `bench/baseline.selfhosted.json`, and
`bench/baseline.hosted.json`.

## Fixture API

The library exports deterministic, source-grounded benchmark support:

| API                           | Purpose                                                        |
| ----------------------------- | -------------------------------------------------------------- |
| `LayerSpec`                   | Describe synthetic layer file count, size, and directory depth |
| `build_layer_tar_gz`          | Build deterministic gzip-compressed tar bytes                  |
| `sha256_digest`               | Produce an OCI-style `sha256:<hex>` digest                     |
| `BenchRegistry`               | Serve one synthetic OCI image through Wiremock                 |
| `is_root`                     | Guard root-required benchmark bodies                           |
| `documented_criterion_group!` | Define documented Criterion harness functions                  |

```rust
use minibox_bench::{LayerSpec, build_layer_tar_gz, sha256_digest};

let layer = build_layer_tar_gz(&LayerSpec {
    file_count: 10,
    file_size_bytes: 4096,
    dir_depth: 2,
});
assert!(sha256_digest(&layer).starts_with("sha256:"));
```

## Development and testing

Fixture tests verify deterministic archives, extraction through the real consumer, digest format,
and an end-to-end pull through `BenchRegistry`.

```bash
cargo check -p minibox-bench --all-targets
cargo clippy -p minibox-bench --all-targets -- -D warnings
cargo nextest run -p minibox-bench
cargo xtask bench --check
```

Do not place Criterion targets in the runtime crates. Keep benchmark-only dependencies and
`test-utils` feature activation isolated here.
