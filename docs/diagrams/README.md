# Minibox Architecture Diagrams

Visual references for minibox design. Each file contains a description, ASCII art, and a Mermaid diagram.

| File                                                           | What it shows                                              |
| -------------------------------------------------------------- | ---------------------------------------------------------- |
| [crate-dependency-graph.md](crate-dependency-graph.md)         | Workspace crate relationships and platform-gated deps      |
| [hexagonal-architecture.md](hexagonal-architecture.md)         | Domain ports, adapters, and composition roots per platform |
| [platform-adapter-selection.md](platform-adapter-selection.md) | Runtime adapter selection flow per platform                |
| [container-lifecycle.md](container-lifecycle.md)               | Container state machine (Created → Running → Stopped)      |
| [miniboxctl-sse-pipeline.md](miniboxctl-sse-pipeline.md)       | HTTP control plane: POST /jobs → daemon → SSE log stream   |
| [env-var-flow.md](env-var-flow.md)                             | How user env vars travel from CLI/miniboxctl to execve(2)  |
| [dagu-minibox-integration.md](dagu-minibox-integration.md)     | Dagu workflow orchestration via minibox-dagu + miniboxctl  |
