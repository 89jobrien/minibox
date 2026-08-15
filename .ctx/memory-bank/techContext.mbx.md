# Tech context

**Stack**

- Language / runtime: Rust 2024 edition, MSRV 1.85
- Framework: tokio (async runtime), axum (HTTP/metrics), clap (CLI)
- Package manager: cargo, workspace resolver v3
- Major dependencies: nix 0.29, reqwest 0.12 (rustls), serde/serde_json,
  flate2/tar (image layers), sha2 (content addressing), chrono, uuid,
  opentelemetry 0.31 + tracing-opentelemetry 0.32 (OTLP/gRPC),
  prometheus-client 0.23, dashmap 6, ipnet 2

**Environment**

- Dev machine: macOS (aarch64-apple-darwin)
- Deploy target: Linux x86_64 (native adapter requires root, kernel 5.0+,
  cgroups v2, overlayfs)
- Cross-compile target: x86_64-unknown-linux-musl (vendored OpenSSL)
- Required env vars: `MINIBOX_ADAPTER` (optional, selects adapter suite),
  `MINIBOX_ALLOW_BIND_MOUNTS`, `MINIBOX_ALLOW_PRIVILEGED` (policy gates),
  `MINIBOX_NETWORK_MODE` (bridge/host/noop)

**Build & test**

- `cargo check --workspace` — compile check
- `cargo xtask verify` — fmt + check + clippy + borrow fixtures + docs lint
- `cargo xtask pre-commit` — macOS pre-commit gate
- `cargo xtask test unit` — cross-platform unit + conformance subset
- `cargo xtask doctor` — canonical preflight (tools, env, Linux system caps; absorbed
  `scripts/preflight.nu`'s checks)
- `just test-integration` — Linux+root cgroup tests
- `just test-e2e` — Linux+root daemon/CLI tests
- `cargo bench -p minibox` — criterion benchmarks

**Constraints**

- Dual license: MIT OR Apache-2.0
- Stabilization freeze — no new features without approval
- Self-hosted GHA runner on VPS (label: `minibox`) for Linux tests
- macOS CI on GitHub-hosted runners
