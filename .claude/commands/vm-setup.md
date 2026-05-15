---
name: vm-setup
description: >
  Bootstrap a Linux VM for minibox development. Installs Rust, just,
  cargo-deny, cargo-audit, and build dependencies. Checks cgroups v2.
argument-hint: ""
allowed-tools: [Bash]
---

Bootstrap a Linux VM (Debian/Ubuntu) for minibox development.

1. **Rust**: if `rustup` absent, install via:
   `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`
   If present: `rustup update stable`

2. **just**: if `just` absent, `cargo install just`

3. **cargo-deny**: if `~/.cargo/bin/cargo-deny` absent, `cargo install cargo-deny`

4. **cargo-audit**: if `~/.cargo/bin/cargo-audit` absent, `cargo install cargo-audit`

5. **Build deps** (if `apt-get` available):
   `sudo apt-get install -y pkg-config libssl-dev`

6. **cgroups v2 check**:
   - Run `mount | grep cgroup2` → print ✓ or ✗
   - Check `cat /proc/filesystems | grep cgroup2` → print ✓ or ✗
   - Print warning (not error) if cgroups v2 absent

Print `setup complete` when done.
