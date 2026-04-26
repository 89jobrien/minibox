# Stability Checklist

Gates that must be green before adding new Core or Platform crates, or
promoting an Experimental crate. See `docs/CRATE_TIERS.md` for the full
stabilization policy.

Last updated: 2026-04-27

---

## Gates

| # | Gate | Status | Evidence |
| - | ---- | ------ | -------- |
| 1 | Protocol types have a single source of truth | Met | `minibox-core/src/protocol.rs` (#122/#128) |
| 2 | Handler coverage >= 80% function coverage | Not met | Current ~67.5% (`handler.rs`) |
| 3 | All wired adapters have at least one integration test | Met | native, gke, colima all tested |
| 4 | `cargo xtask pre-commit` passes on macOS | Met | fmt + clippy + release build |
| 5 | `cargo xtask test-unit` passes | Met | ~760 tests |
| 6 | `cargo deny check` passes | Met | License + advisory audit in CI |

---

## How to Verify

```bash
# Gate 1: protocol snapshot tests
cargo test -p minibox-core -- protocol

# Gate 2: handler coverage (requires Linux + llvm-cov)
cargo xtask prepush  # generates coverage report

# Gate 3: adapter integration tests
just test-integration  # Linux + root
just test-adapters     # Colima + handler adapter swap

# Gate 4: pre-commit gate
cargo xtask pre-commit

# Gate 5: unit test suite
cargo xtask test-unit

# Gate 6: deny + audit
cargo deny check
cargo audit
```

---

## Freeze Status

The stabilization freeze (issues #117 and #127) applies to **net-new Core
and Platform crates**. The freeze lifts when all six gates above are verified
green on the `next` branch.

Gate 2 (handler coverage) is the primary remaining blocker. See
[GH #158](https://github.com/89jobrien/minibox/issues/158) for tracking.
