# ail

`ail` is a reserved binary crate for a future agent-improvement-loop runner.

## Status

This crate is a placeholder, not an implemented workflow engine. The current binary has no
arguments, configuration, library API, or dependencies. Running it prints a placeholder message:

```bash
cargo run -p ail
```

Do not build automation around its current output; it is not a stable protocol or CLI contract.
Agent improvement workflows currently live outside this crate.

## Architecture role

`ail` is a workspace leaf and does not participate in the minibox daemon, protocol, adapter, or
container-runtime dependency rings. Its presence reserves the package and binary name without
coupling unfinished agent orchestration to production runtime crates.

## Development and testing

There are currently no crate-specific tests. Workspace checks still compile and lint the binary:

```bash
cargo check -p ail
cargo clippy -p ail --all-targets -- -D warnings
cargo xtask verify
```

Any future implementation should add CLI help, tests, and a documented execution model before the
binary is treated as usable.
