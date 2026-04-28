---
status: open
spec: docs/superpowers/specs/2026-04-26-winbox-wsl2-proxy-design.md
scope: crates/winbox/ + crates/miniboxd/ + crates/mbx/
issues: ["#45", "#87"]
---

# winbox WSL2 Proxy -- Plan Stub

Spec exists but no implementation plan has been written. The spec
describes a Named Pipe server in winbox that proxies to a WSL2-hosted
miniboxd instance.

## Prerequisites

- Windows phase 2 prioritization decision
- WSL2 distro selection and testing environment
