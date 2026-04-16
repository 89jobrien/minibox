# Changelog

- 2026-04-16: Fixed invalid GitHub Actions `ci.yml` syntax, made `ProotRuntime::from_env()` fail on bad `MINIBOX_PROOT_PATH` overrides, and isolated env-sensitive `minibox-llm` tests from ambient provider keys.
- 2026-04-16: Pinned the Rust toolchain to `1.91.1` to stop CI `rustfmt` drift and rewired stale `daemonbox` tests to the current `registry_router`-based `HandlerDependencies` API.
