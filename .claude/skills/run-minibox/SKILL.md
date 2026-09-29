---
name: run-minibox
description: Build, launch, and drive minibox (miniboxd + mbx CLI) end-to-end — start an isolated daemon, pull an image, run a container, inspect ps/logs, and tear down. Use when asked to run minibox, start miniboxd, smoke-test mbx, or verify a change actually works via a real container lifecycle rather than just `cargo test`.
---

# run-minibox

Minibox is a daemon/CLI pair: `miniboxd` (daemon, talks to a container
backend adapter) and `mbx` (CLI, talks to `miniboxd` over a Unix socket).
There is no GUI — the "app" is a socket protocol, so the driver here is a
Nushell wrapper (`driver.nu`) that boots an **isolated** `miniboxd` (its own
socket + state dir, independent of any system-level minibox daemon) and
drives it with the real `mbx` binary. Verified 2026-08-23 on macOS (Darwin
25.4.0) using the `smolvm` adapter (default; boots a lightweight Linux VM
per-run) — this is the adapter `cargo xtask doctor` recommends on this host.
Paths below are relative to the minibox repo root
(`/Users/joe/dev/minibox`), not to this skill directory.

## Prerequisites

Nothing beyond what's already on this machine: `cargo`, `rustup`. Verified
via `cargo xtask doctor` — all checks passed, `smolvm` on PATH, no extra
`apt`/`brew` packages were needed.

## Build

```text
cd /Users/joe/dev/minibox
cargo build --release -p miniboxd -p minibox-cli
```

The CLI Cargo package is named `minibox-cli`; its binary remains `mbx`.
Binaries land at `target/release/miniboxd` and `target/release/mbx`.

## Run (agent path) — use the driver

The driver lives at
`.claude/skills/run-minibox/driver.nu` and wraps every step below. It uses a
system-level detached shell (`sh -c '... &'`) to launch `miniboxd`, because
Nushell's `job spawn` and trailing `&` are both tied to the parent `nu -c`
process — the daemon dies as soon as that process exits, which looks like a
successful start followed by silent failure on the next command. Always
launch via the driver, not a raw `job spawn`/`&`.

```text
cd /Users/joe/dev/minibox

# 1. Build (only needed after code changes)
nu -c 'source .claude/skills/run-minibox/driver.nu; build'

# 2. Boot an isolated daemon (socket + state dir under /tmp/mbx-run)
nu -c 'source .claude/skills/run-minibox/driver.nu; start'

# 3. Confirm the adapter it picked and that the socket answers
nu -c 'source .claude/skills/run-minibox/driver.nu; status'

# 4. Full smoke: pull alpine, run a command in a real VM, print output
nu -c 'source .claude/skills/run-minibox/driver.nu; smoke'

# Individual verbs, once a daemon is started:
nu -c 'source .claude/skills/run-minibox/driver.nu; run-cmd alpine:latest echo hi'
nu -c 'source .claude/skills/run-minibox/driver.nu; ps-list'
nu -c 'source .claude/skills/run-minibox/driver.nu; logs <name-or-id>'
nu -c 'source .claude/skills/run-minibox/driver.nu; stop <name-or-id>'
nu -c 'source .claude/skills/run-minibox/driver.nu; rm-container <name-or-id>'

# 5. Kill the isolated daemon and remove /tmp/mbx-run
nu -c 'source .claude/skills/run-minibox/driver.nu; teardown'
```

Actual verified transcript of step 4 (`smoke`):

```text
--- pull alpine:latest ---
Pulling alpine:latest…
pulled alpine:latest
--- run: echo through the VM ---
d3796ceade0649fd
hello-from-mbx
--- ps (should be empty: run is ephemeral, container is auto-removed on exit) ---
CONTAINER ID    NAME              IMAGE                 COMMAND               STATE       CREATED                    PID
-----------------------------------------------------------------------------------------------------------------------------
(no containers)
```

`mbx run` has no `-d`/detach flag — every run is foreground and ephemeral
unless you separately `--name` it and it's still running when you `ps`
(long-lived commands, not `echo`). That's expected, not a driver bug.

## Direct invocation (no daemon)

Most `mbx` subcommands require a live `miniboxd`. The one exception:
`mbx doctor` shells out to `cargo xtask doctor` and reports adapter
capability without touching a socket — useful for inspecting compiled adapter
support before spending time booting anything.

```text
target/release/mbx doctor
```

## Run (human path)

```text
cargo build --release -p miniboxd -p minibox-cli
MINIBOX_SOCKET_PATH=/tmp/miniboxd.sock MINIBOX_DATA_DIR=/tmp/mbx-state ./target/release/miniboxd &
MINIBOX_SOCKET_PATH=/tmp/miniboxd.sock ./target/release/mbx pull alpine:latest
MINIBOX_SOCKET_PATH=/tmp/miniboxd.sock ./target/release/mbx run alpine:latest -- sh
```

Same env vars the driver uses (`MINIBOX_SOCKET_PATH`, `MINIBOX_DATA_DIR`).
Difference from the driver: no automatic detach, no isolated run dir, and
you're responsible for killing the daemon yourself.

## Test

```text
cargo xtask verify           # fmt check, clippy -D warnings, borrow fixtures
cargo xtask test unit        # cross-platform workspace library tests
```

## Gotchas

- **`job spawn` / trailing `&` don't survive `nu -c` exiting.** Both are
  children of the `nu -c` process itself; when a separate `nu -c 'start'`
  call finishes, the daemon it spawned dies with it, and the next `nu -c`
  call sees `Connection refused (os error 61)` against a socket that
  briefly existed. Confirmed by watching `ps | where name =~ "miniboxd"`
  go from present to absent the instant the parent `nu -c` returned. Fix:
  detach via a real subshell (`^sh -c "nohup <bin> ... &"`), which
  reparents to pid 1 and survives.
- **`$env.X = "..."` inside a double-quoted `nu -c "..."` argument gets
  eaten by the outer shell.** This session's outer shell is itself Nushell
  (`nu-login`), and `"..."` still does `$`-interpolation there — `$env.FOO`
  silently becomes `.FOO` (parse error: "Assignment operations require a
  variable"). Use single-quoted `nu -c '...'` for any script containing
  `$env`, `$in`, or other `$`-prefixed nu syntax.
- **Package and binary names differ.** Cargo commands use package
  `minibox-cli` (`crates/mbx/Cargo.toml`), while the executable is named
  `mbx`. The workspace also has `crates/minibox` (core lib) and
  `crates/minibox-core`; neither produces the CLI binary.
- **A local `export def rm [...]` in a Nushell script shadows the builtin
  `rm`.** Defining `rm` to mean "remove a container" broke `rm -r -f
$RUN_DIR` in `teardown` — it called the container-removal command
  instead, which requires a running daemon and a container arg it didn't
  have. Renamed to `rm-container`; keep app-verb command names out of
  Nushell's core namespace (`rm`, `ps`, `open`, `run`, ...).
- **`nu`'s `kill` needs an int, not whatever `open` gives you back from a
  plain-text pid file.** `open pidfile.txt | kill` (or `kill $p` where `$p`
  came from `open`) silently fails and falls into the `catch` branch even
  though the process is alive — `open` returns a string for extension-less
  files, and `kill` type-mismatches without a loud error. Pipe through
  `into int` first.
- **`ps -a` is not a valid `mbx ps` flag.** `mbx ps` already lists every
  container regardless of state; passing `-a` exits 2 with no output
  (clap's unknown-flag error goes to stderr, and a naive `.stdout`-only
  check makes it look like `ps` silently returned nothing). Just run
  `mbx ps`.
- **Ephemeral runs are correct, not a bug.** `mbx run` has no detach flag.
  A `pull` → `run --name X ...` → `ps` sequence will show `(no
containers)` for `ps` if the run's command already exited — that's
  cleanup working, not the container vanishing unexpectedly.

## Troubleshooting

| Symptom                                                                          | Fix                                                                                                                                      |
| -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `Connection refused (os error 61)` from any `mbx` command                        | The daemon is not running on that socket. Re-run `start`; the driver now fails unless the expected socket appears.                       |
| `metrics server bind ... Address already in use (os error 48)` in the daemon log | Harmless for container lifecycle testing: the daemon logs a `WARN` and continues without metrics because another process owns port 9090. |
| `error: package ID specification 'mbx' did not match any packages`               | Cargo uses the package name `minibox-cli`; run `cargo build -p minibox-cli`. The resulting binary is still `target/release/mbx`.         |
| `mbx doctor` prints `cargo xtask doctor` output instead of daemon status         | Expected: `doctor` is a preflight/capability check, not a socket call. Use `mbx ps` to exercise the running daemon.                      |
