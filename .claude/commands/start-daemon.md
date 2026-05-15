---
name: start-daemon
description: >
  Start miniboxd from the release binary with --restart. Optionally select
  the adapter suite via --adapter. Use for local dev and smoke testing.
argument-hint: "[--adapter <colima|smolvm|krun|native|gke>]"
---

# start-daemon

Starts the locally built `miniboxd` with `--restart`.

```nu
nu scripts/start-daemon.nu                    # auto-select adapter
nu scripts/start-daemon.nu --adapter smolvm   # explicit adapter
nu scripts/start-daemon.nu --adapter colima
nu scripts/start-daemon.nu --adapter native   # Linux only
```

Requires a release build at `~/.minibox/cache/target/release/miniboxd`.
Build with: `cargo build --release -p miniboxd`

The `MINIBOX_ADAPTER` env var has the same effect as `--adapter`.
