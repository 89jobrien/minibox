# Adapter Composition

`miniboxd` is the composition root for native, GKE, Colima, smolvm, and krun suites, with a feature-gated VZ path and a Windows dispatch to the `winbox` stub. Each suite supplies the domain ports grouped by `HandlerDependencies`.

See [[Minibox]], [[Hexagonal Architecture]], and [[Daemon Protocol]].

Evidence: `crates/miniboxd/src/main.rs:705-1133`, `crates/minibox/src/daemon/handler/mod.rs:155-282`.
