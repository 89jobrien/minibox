---
name: minibox
description: >
  Minibox development skills — performance analysis, protocol sync audits,
  and release workflows for the minibox container runtime.
---

# minibox

Development skills and reference docs for the minibox container runtime.

## Reference Docs

| Doc | Purpose |
| --- | ------- |
| [performance](references/performance.md) | Container init latency, codec/adapter benchmarks, memory profiling |
| [protocol-sync](references/protocol-sync.md) | DaemonRequest propagation audit across 6 sites |
| [release](references/release.md) | Quality gates, version bump, changelog, git tag, push |

## Related Skills

| Skill | Purpose |
| --- | --- |
| `changed-files-secret-scan` | Redacted TruffleHog and Gitleaks scans limited to changed and untracked files before staging or committing |
