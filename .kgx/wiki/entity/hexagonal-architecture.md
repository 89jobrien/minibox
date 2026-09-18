# Hexagonal Architecture

The enforced inner-to-outer rings are `minibox-domain` -> `minibox-core` -> `minibox`. Domain ports are injected through focused `HandlerDependencies` groups, and architecture checks reject outward dependencies and shadow ownership.

See [[Minibox]] and [[Adapter Composition]].

Evidence: `xtask/src/architecture.rs:68-187`, `crates/minibox/src/daemon/handler/mod.rs:155-282`.
