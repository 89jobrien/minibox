# Bind Mounts, Privileged Mode, and DinD Support

**Date:** 2026-03-27
**Status:** Approved

## Goal

Enable minibox-in-minibox (DinD) so that macOS can run a Linux container via the Colima adapter, and inside that container run `miniboxd` under `uftrace` for function-level tracing. This requires two new features: bind mounts (to inject binaries and extract trace data) and privileged mode (to grant the inner `miniboxd` the Linux capabilities it needs to create namespaces and run containers).

## Scope

- Bind mounts: `-v src:dst[:ro]` shorthand and `--mount type=bind,...` long form
- Privileged mode: `--privileged` flag granting full Linux capability set
- Cross-compilation: `just build-linux` producing static musl Linux binaries from any host
- Updated `just trace`: works on macOS via minibox + Colima, and on Linux via native uftrace

Out of scope: named volumes, tmpfs mounts, per-capability `--cap-add`/`--cap-drop`, user namespace remapping, networking changes.

## Architecture

Approach B (full native implementation) — the protocol is the contract. Both the native Linux adapter and the Colima adapter conform to the same protocol structs. Behaviour is consistent across hosts.

```
CLI parse (-v / --mount / --privileged)
    │
    ▼
DaemonRequest::Run { mounts: Vec<BindMount>, privileged: bool, ... }
    │
    ▼
handler.rs → run_inner()
    │
    ├─ native adapter (linuxbox)
    │       ├─ filesystem.rs: MS_BIND mounts before pivot_root
    │       └─ process.rs: capset(all) in child if privileged
    │
    └─ colima adapter
            └─ nerdctl run -v ... [--privileged]
```

## Section 1 — Protocol Layer

**File:** `crates/minibox-core/src/protocol.rs`

New types:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindMount {
    pub host_path: PathBuf,
    pub container_path: PathBuf,
    pub read_only: bool,
}
```

`DaemonRequest::Run` gains (both `#[serde(default)]` for backwards compatibility):

```rust
#[serde(default)]
pub mounts: Vec<BindMount>,
#[serde(default)]
pub privileged: bool,
```

**File:** `crates/minibox-core/src/domain.rs`

`ContainerSpawnConfig` gains the same two fields so the handler can pass them through the `ContainerRuntime` trait to each adapter.

## Section 2 — Native Linux Adapter

### Bind Mounts (`crates/linuxbox/src/container/filesystem.rs`)

After the overlay is mounted and before `pivot_root`, each `BindMount` is applied inside the container's new mount namespace:

1. Validate `host_path`: `canonicalize` + confirm it does not escape outside `/`.
2. Construct `target = rootfs.join(container_path.strip_prefix("/"))`.
3. `create_dir_all(target)` if absent (mirrors Docker behaviour).
4. `mount(host_path, target, MS_BIND | MS_REC)`.
5. If `read_only`: `mount("", target, MS_BIND | MS_RDONLY | MS_REMOUNT)`.

Error on any failure; clean up already-applied mounts before returning (best-effort, warn on secondary error).

### Privileged Mode (`crates/linuxbox/src/container/process.rs`)

`NamespaceConfig` gains `user_namespace: bool`. Privileged containers set this to `false` so they inherit the parent's (root) capability set rather than starting with a reduced set inside a user namespace.

In the child process, after `clone()` and before `execvp()`, if `privileged: true`:

1. `prctl(PR_SET_KEEPCAPS, 1)` — retain capabilities across the UID transition.
2. Build a `libc::__user_cap_data_struct` with all bits set in `permitted`, `effective`, and `inheritable`.
3. `capset(2)` with the full capability set.

This grants `CAP_SYS_ADMIN`, `CAP_SYS_CHROOT`, `CAP_NET_ADMIN`, `CAP_MKNOD`, and all others required for DinD without the caller needing to enumerate them.

`ContainerConfig` in `process.rs` gains `privileged: bool` and `mounts: Vec<BindMount>`.

## Section 3 — Colima Adapter

**File:** `crates/linuxbox/src/adapters/colima.rs`

The `create` method translates `ContainerSpawnConfig` fields to `nerdctl run` flags:

```rust
fn mount_flag(m: &BindMount) -> String {
    let ro = if m.read_only { ":ro" } else { "" };
    format!("-v {}:{}{ro}", m.host_path.display(), m.container_path.display())
}

if config.privileged {
    cmd.arg("--privileged");
}
for mount in &config.mounts {
    cmd.arg(mount_flag(mount));
}
```

**Lima path check:** Before building the nerdctl command, validate that every bind mount `host_path` is under `$HOME` or `/tmp`. Lima only shares these directories into the VM by default; mounts outside them will silently fail. Return a clear error if the check fails:

```
error: bind mount source '/opt/foo' is not accessible inside the Lima VM.
hint: Lima shares $HOME and /tmp — move the source path or add it to lima.yaml shared dirs.
```

Host path validation (canonicalize + no-escape check) is performed in `run_inner` before reaching the adapter, so the Colima adapter trusts the paths are safe.

## Section 4 — CLI

**File:** `crates/minibox-cli/src/main.rs` (or `commands/run.rs`)

New flags on `minibox run`:

```
--privileged
    Grant full Linux capabilities to the container (required for DinD).

-v, --volume <src:dst[:ro]>
    Bind mount host path into container. Repeatable.
    Example: -v $(pwd)/bin:/minibox  -v $(pwd)/traces:/traces:ro

--mount <type=bind,src=<path>,dst=<path>[,readonly]>
    Long-form mount specification. Repeatable.
    Example: --mount type=bind,src=./bin,dst=/minibox
```

Both `-v` and `--mount` parse into `BindMount`. `-v` is shorthand that expands to the same struct. Parse errors (missing `:`, non-existent host path, relative `dst`) are caught before the socket is touched.

## Section 5 — Cross-Compilation

**File:** `Justfile`

```just
# Build static Linux x86_64 binaries (works from macOS or Linux)
# Requires: rustup target add x86_64-unknown-linux-musl
build-linux:
    rustup target add x86_64-unknown-linux-musl
    RUSTFLAGS="-C target-feature=+crt-static" \
        cargo build --release --target x86_64-unknown-linux-musl \
        -p miniboxd -p minibox-cli
```

Output: `target/x86_64-unknown-linux-musl/release/miniboxd` and `minibox`.

## Section 6 — Updated `just trace`

```just
# Trace daemon with uftrace.
# macOS: runs inside a minibox Linux container via Colima (requires miniboxd running).
# Linux: runs natively (requires root + uftrace installed).
trace:
    #!/usr/bin/env bash
    set -euo pipefail

    TRACE_DIR="traces/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$TRACE_DIR"

    if [[ "$(uname -s)" == "Darwin" ]]; then
        just build-linux
        minibox run --privileged \
            -v "$(pwd)/target/x86_64-unknown-linux-musl/release:/minibox" \
            -v "$(pwd)/$TRACE_DIR:/traces" \
            ubuntu \
            -- sh -c "apt-get install -y uftrace -q && \
                      uftrace record -P . --no-libcall -d /traces /minibox/miniboxd"
        uftrace report -d "$TRACE_DIR" --sort=total | head -25
    else
        [[ "$(uname -s)" == "Linux" ]] || { echo "error: unsupported platform"; exit 1; }
        command -v uftrace >/dev/null 2>&1 || { echo "error: apt install uftrace"; exit 1; }
        [[ "$(id -u)" -eq 0 ]] || { echo "error: sudo just trace"; exit 1; }
        cargo build --release -p miniboxd -p minibox-cli
        uftrace record -P . --no-libcall -d "$TRACE_DIR" ./target/release/miniboxd &
        DAEMON_PID=$!
        for i in $(seq 1 10); do [[ -S /run/minibox/miniboxd.sock ]] && break; sleep 0.5; done
        [[ -S /run/minibox/miniboxd.sock ]] || { kill "$DAEMON_PID" 2>/dev/null; exit 1; }
        ./target/release/minibox pull alpine || true
        ./target/release/minibox run alpine -- /bin/echo "uftrace smoke" || true
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
        uftrace report -d "$TRACE_DIR" --sort=total | head -25
    fi

    echo ""
    echo "trace: data saved to $TRACE_DIR"
    echo "trace: call graph      → uftrace graph -d $TRACE_DIR"
    echo "trace: chrome devtools → uftrace dump -d $TRACE_DIR --chrome > $TRACE_DIR/trace.json"
```

## Error Handling

| Scenario | Behaviour |
|---|---|
| Bind mount host path does not exist | CLI rejects at parse time with path shown |
| Bind mount host path outside Lima share dirs (Colima) | Adapter returns error before nerdctl is called |
| Privileged + no root on native | `capset` fails; child exits; daemon surfaces error to CLI |
| Bind mount target creation fails in container | `create_dir_all` error propagated; overlay cleanup runs |
| Partial bind mount failure mid-sequence | Already-applied bind mounts unmounted (best-effort, warn on failure) |

## Testing

- **Unit:** Parse tests for `-v` and `--mount` covering valid, read-only, missing colon, relative dst, non-existent src.
- **Unit:** `filesystem.rs` bind mount logic with a tmpdir rootfs and tmpdir host source; verify MS_BIND applied and read-only remount works.
- **Unit:** `process.rs` privileged flag wires through to `NamespaceConfig` correctly.
- **Integration (Linux + root):** Container with `-v` and a file written by the host; verify file visible inside. Container with `--privileged`; verify `CAP_SYS_ADMIN` present via `capsh --print`.
- **E2E (Linux + root):** Full DinD smoke — outer minibox starts inner miniboxd which pulls and runs alpine echo.
- **Colima (macOS, manual):** `just trace` end-to-end produces a non-empty `traces/` directory.
