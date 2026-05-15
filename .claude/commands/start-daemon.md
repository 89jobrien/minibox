---
name: start-daemon
description: >
  Start miniboxd from the release binary with --restart. Optionally select
  the adapter suite via --adapter. Use for local dev and smoke testing.
argument-hint: "[--adapter <colima|smolvm|krun|native|gke>]"
allowed-tools: [Bash]
---

Start miniboxd with `--restart`. Parse `$ARGUMENTS` for: `--adapter <value>`.

1. Resolve binary: `~/.minibox/cache/target/release/miniboxd`
2. If binary does not exist: error with
   `miniboxd not found — run: cargo build --release -p miniboxd`
3. If `--adapter <value>` provided: set `MINIBOX_ADAPTER=<value>` in env
4. Print: `Starting miniboxd (MINIBOX_ADAPTER=<adapter|auto>) with --restart...`
5. Run: `<binary-path> --restart` (with MINIBOX_ADAPTER in env if set)

Adapter choices: colima, smolvm, krun, native, gke. Default: auto (smolvm, falls back to krun).
