---
name: vm-setup
description: >
  Bootstrap a Linux VM for minibox development. Installs Rust, just,
  cargo-deny, cargo-audit, and build dependencies. Checks cgroups v2.
argument-hint: ""
---

# vm-setup

Sets up a fresh Linux VM (Debian/Ubuntu) for minibox development.

```nu
nu scripts/vm-setup.nu
```

Installs:
- Rust (via rustup) or runs `rustup update stable` if already present
- `just`
- `cargo-deny`
- `cargo-audit`
- `pkg-config`, `libssl-dev` (via apt-get if available)

Also verifies that cgroups v2 is mounted and kernel-supported. Exits with a
warning (not error) if cgroups v2 is absent.

Intended for use on the minibox VPS or a fresh CI runner — not macOS.
