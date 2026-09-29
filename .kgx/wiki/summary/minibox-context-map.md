# Minibox Context Map

Minibox is a 17-member Rust workspace organized around enforced hexagonal dependency rings: `minibox-domain` owns ports and values, `minibox-core` owns shared protocol/client infrastructure, and `minibox` owns daemon/runtime behavior. `miniboxd` composes platform adapters, while `mbx`, MCP, Crux, and TUI all use the same daemon protocol. Conformance and repository gates are owned by `minibox-testsuite` and `xtask`.

See [[Minibox]], [[Hexagonal Architecture]], [[Daemon Protocol]], and [[Adapter Composition]].

Evidence: `Cargo.toml:3`, `xtask/src/architecture.rs:68`, `crates/minibox-core/src/protocol.rs:104`, `crates/miniboxd/src/main.rs:705` at commit `f5481a94`.
