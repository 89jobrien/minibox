---
name: meta-agent
description: >
  Design and spawn parallel Claude agents from a task description. Fetches and
  caches Claude Agent SDK docs (24h TTL), designs 2–5 agents, runs concurrently,
  and synthesises results.
argument-hint: "<task> [--no-synthesis] [--refresh-docs]"
---

# meta-agent

Decomposes a task into parallel sub-agents, runs them concurrently, and
synthesises their output.

```nu
nu scripts/meta-agent.nu "add overlay cleanup on container stop"
nu scripts/meta-agent.nu "audit error handling in adapters" --no-synthesis
nu scripts/meta-agent.nu "review protocol changes" --refresh-docs
echo "fix cgroup teardown" | nu scripts/meta-agent.nu
```

Flags:
- `--no-synthesis` — print each agent's output separately, skip the final synthesis
- `--refresh-docs` — force re-fetch of Claude Agent SDK docs even if cache is fresh
