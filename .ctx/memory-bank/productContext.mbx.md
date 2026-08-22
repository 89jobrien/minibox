# Product context

- **Problem**: Most container runtimes are large, opaque, and hard to embed or
  extend. Minibox is intentionally small and readable — every layer (protocol,
  domain traits, adapters, daemon) is swappable.
- **Primary users**: The author (learning + reference), AI agent workloads
  (agentbox design), anyone wanting a minimal embeddable container runtime.
- **UX principles**:
  - Docker-compatible CLI surface (`pull`, `run`, `stop`, `rm`, `ps`, `logs`,
    `exec`, `events`, `pause`, `resume`, `prune`, `rmi`)
  - Daemon/CLI split over Unix socket
  - Structured tracing (OTLP/gRPC export)
  - Adapter suites selectable at startup via env var, no recompile
- **Constraints**:
  - Linux: root, kernel 5.0+, cgroups v2, overlayfs
  - macOS: requires smolvm or krun (VM-backed); exec/logs not supported
