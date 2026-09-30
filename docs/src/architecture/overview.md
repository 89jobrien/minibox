# Architecture Overview

Minibox uses enforced dependency rings: `minibox-domain` owns pure values and
ports, `minibox-core` adds protocol/client/OCI infrastructure, and `minibox`
owns runtime adapters plus daemon behavior. `miniboxd` is the composition root.

Platform crates (`macbox`, `smolbox`, and `winbox`) implement host-specific
behavior. The `mbx`, MCP, Crux, and TUI frontends all use the canonical daemon
protocol from `minibox-core`.

See [`docs/core/ARCHITECTURE.mbx.md`](../../core/ARCHITECTURE.mbx.md) for the
full dependency graph, port inventory, adapter matrix, and lifecycle flow.
