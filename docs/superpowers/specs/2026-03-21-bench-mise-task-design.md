# Benchmark Mise Task Design

**Date:** 2026-03-21
**Status:** Approved
**Goal:** Add `mise run bench:setup` and `mise run bench` tasks — compile and run `minibox-bench` on the VPS via SSH, post results as a Gitea commit comment.

---

## Background

`crates/minibox-bench/` is a fully-implemented Rust binary that shells out to `minibox` CLI, runs suites (pull, run, exec, e2e), and writes JSON + text results to a timestamped output directory. Currently there is no automated way to run it and publish results. The VPS CI pipeline is restricted to `cargo deny` + `cargo audit` (no compilation), so benchmarks cannot run in CI on every push.

The solution is a `mise run bench` task that SSHes into the VPS, runs the pre-compiled binary, captures the text table, and posts it as a Gitea commit comment — making benchmark results visible in the commit history without touching the CI pipeline.

---

## Architecture

Two new tasks in `mise.toml`:

```
mise run bench:setup   — one-time setup: clone repo + compile binary on VPS
mise run bench         — regular run: pre-flight → run → post Gitea comment
```

Both use the same SSH pattern as the existing `mise run ci` task:
- VPS password from 1Password (`op item get jobrien-vm`)
- `sshpass` + SSH options for password auth
- Short-lived Gitea API token generated via `gitea admin user generate-access-token`, cleaned up on exit

---

## Task Designs

### `bench:setup`

**Responsibility:** One-time setup. Clone the repo on the VPS and compile the bench binary.

**Steps:**
1. Fetch VPS password from 1Password
2. SSH in as `dev@100.105.75.7`
3. Clone `http://100.105.75.7:3000/joe/minibox` to `~/minibox` if absent; `git pull` if present
4. From `~/minibox`, run `~/.local/bin/mise exec -- cargo build --release -p minibox-bench`
5. Confirm `~/minibox/target/release/minibox-bench` exists and print confirmation

**Expected runtime:** Several minutes (first run). Subsequent runs after `git pull` are incremental.

---

### `bench`

**Responsibility:** Run benchmark, capture output, post as Gitea commit comment.

**Flow:**

```
1. Fetch VPS password from 1Password
2. Generate short-lived Gitea token
3. SSH in — single bash session, stdout captured via command substitution
   (same pattern as `mise run ci`: `BENCH_OUT=$(sshpass ... ssh ... 'bash -s' <<'ENDSSH' ... ENDSSH)`):
   a. Pre-flight checks (fail fast with clear message):
      - ~/minibox/target/release/minibox-bench exists
        → else: "minibox-bench not found — run: mise run bench:setup"
      - minibox in PATH
        → else: "minibox not installed on VPS"
      - /run/minibox/miniboxd.sock exists
        → else: "miniboxd not running — start the daemon first"
   b. Remove any stale output dir: rm -rf /tmp/bench-out-$$
   c. Run: minibox-bench --iters 5 --out-dir /tmp/bench-out-$$
      (5 is intentional — pull/e2e are single-shot; run/exec at 5 iters
      takes ~30–60 s on the VPS vs ~4 min at the default 20)
   d. Find the .txt result: ls -t /tmp/bench-out-$$/*.txt | head -1, then cat it
   e. Clean up /tmp/bench-out-$$
4. Capture txt output locally
5. Get commit SHA: git rev-parse HEAD (local)
6. POST comment to Gitea commits/{sha}/comments
7. Clean up Gitea token (trap EXIT)
```

**Pre-flight failures** exit with code 1 and a message telling the user exactly what to fix. They do not generate a Gitea token.

**Comment format:**

~~~markdown
## Benchmark Results

**Host:** jobrien-vm | **Commit:** {sha[:8]}

```
{txt table output}
```
~~~

**Gitea API endpoint:** `POST /api/v1/repos/joe/minibox/commits/{sha}/comments`

---

## Error Handling

| Failure | Behaviour |
|---------|-----------|
| 1Password lookup fails | `exit 1` with message |
| SSH connection fails | `exit 1` — sshpass/ssh error propagates |
| Binary not found | `exit 1`: "run `mise run bench:setup`" |
| minibox not in PATH | `exit 1`: "minibox not installed on VPS" |
| miniboxd socket absent | `exit 1`: "miniboxd not running" |
| bench binary fails | `exit 1` — stderr from bench propagates |
| Gitea comment POST fails | Print warning, don't crash (results already printed locally) |

---

## File Map

| Action | File |
|--------|------|
| Modify | `mise.toml` — add `bench:setup` and `bench` tasks |
