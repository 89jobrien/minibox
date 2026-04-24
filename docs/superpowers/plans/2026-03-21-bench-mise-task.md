---
status: done
---

# Bench Mise Task Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `mise run bench:setup` and `mise run bench` tasks to `mise.toml` — the former compiles `minibox-bench` on the VPS once; the latter runs it, captures the text table, and posts it as a Gitea commit comment.

**Architecture:** Two bash tasks in `mise.toml` following the existing `mise run ci` pattern: VPS password from 1Password, `sshpass` for SSH, short-lived Gitea token generated via `gitea admin user generate-access-token` with `trap cleanup EXIT`. `bench` captures SSH stdout via command substitution; builds the comment body with Python's `json.dumps` to avoid JSON injection; posts to `POST /api/v1/repos/joe/minibox/commits/{sha}/comments`.

**Tech Stack:** Bash, sshpass, curl, python3 (JSON body construction), Gitea REST API v1, 1Password CLI (`op`).

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `mise.toml` | Add `bench:setup` and `bench` tasks |

---

## Task 1: `bench:setup` — clone repo and compile binary on VPS

**Files:**
- Modify: `mise.toml`

- [ ] **Step 1: Add the `bench:setup` task**

Append to `mise.toml` after the existing `ci:set-secret` task:

```toml
[tasks."bench:setup"]
description = "One-time setup: clone minibox repo on VPS and compile minibox-bench"
run = """
#!/usr/bin/env bash
set -euo pipefail

VPS_PASS=$(op item get jobrien-vm --account=my.1password.com --fields password --reveal 2>/dev/null)
SSH_OPTS="-o IdentitiesOnly=yes -o IdentityAgent=none -o PreferredAuthentications=password -o StrictHostKeyChecking=no"

sshpass -p "$VPS_PASS" ssh $SSH_OPTS dev@100.105.75.7 'bash -s' <<'ENDSSH'
set -euo pipefail

REPO_DIR=~/minibox

if [ -d "$REPO_DIR/.git" ]; then
  echo "Pulling latest..."
  git -C "$REPO_DIR" pull
else
  echo "Cloning repo..."
  git clone http://100.105.75.7:3000/joe/minibox "$REPO_DIR"
fi

echo "Building minibox-bench..."
cd "$REPO_DIR"
~/.local/bin/mise exec -- cargo build --release -p minibox-bench

echo "✓ minibox-bench ready at $REPO_DIR/target/release/minibox-bench"
ENDSSH
"""
```

- [ ] **Step 2: Verify the task is visible**

```bash
mise tasks | grep bench
```

Expected output includes `bench:setup`.

- [ ] **Step 3: Commit**

```bash
git add mise.toml
git commit -m "feat(mise): add bench:setup — clone repo and compile minibox-bench on VPS"
```

---

## Task 2: `bench` — run benchmark and post Gitea commit comment

**Files:**
- Modify: `mise.toml`

- [ ] **Step 1: Add the `bench` task**

Append to `mise.toml` after `bench:setup`:

```toml
[tasks.bench]
description = "Run minibox-bench on VPS and post results as a Gitea commit comment"
run = """
#!/usr/bin/env bash
set -euo pipefail

GITEA_URL="http://100.105.75.7:3000"
REPO="joe/minibox"
COMMIT_SHA=$(git rev-parse HEAD)

VPS_PASS=$(op item get jobrien-vm --account=my.1password.com --fields password --reveal 2>/dev/null)
SSH_OPTS="-o IdentitiesOnly=yes -o IdentityAgent=none -o PreferredAuthentications=password -o StrictHostKeyChecking=no"
TOKEN_NAME="bench-$$"

TOKEN=$(sshpass -p "$VPS_PASS" ssh $SSH_OPTS dev@100.105.75.7 \
  "echo '$VPS_PASS' | sudo -Su git /usr/local/bin/gitea admin user generate-access-token \
    --username joe --token-name $TOKEN_NAME --raw \
    --config /var/lib/gitea/custom/conf/app.ini 2>/dev/null")

[[ -z "$TOKEN" ]] && { echo "error: could not generate Gitea token" >&2; exit 1; }

cleanup() {
  curl -sf -X DELETE -H "Authorization: token $TOKEN" \
    "$GITEA_URL/api/v1/users/joe/tokens/$TOKEN_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

BENCH_OUT=$(sshpass -p "$VPS_PASS" ssh $SSH_OPTS dev@100.105.75.7 'bash -s' <<'ENDSSH'
set -euo pipefail

BENCH_BIN=~/minibox/target/release/minibox-bench

if [ ! -f "$BENCH_BIN" ]; then
  echo "error: minibox-bench not found — run: mise run bench:setup" >&2
  exit 1
fi

if ! command -v minibox &>/dev/null; then
  echo "error: minibox not installed on VPS" >&2
  exit 1
fi

if [ ! -S /run/minibox/miniboxd.sock ]; then
  echo "error: miniboxd not running — start the daemon first" >&2
  exit 1
fi

OUT_DIR="/tmp/bench-out-$$"
rm -rf "$OUT_DIR"
"$BENCH_BIN" --iters 5 --out-dir "$OUT_DIR"
ls -t "$OUT_DIR"/*.txt | head -1 | xargs cat
rm -rf "$OUT_DIR"
ENDSSH
)

echo "$BENCH_OUT"

COMMENT_BODY=$(python3 -c "
import json, sys
sha = sys.argv[1]
out = sys.argv[2]
body = '## Benchmark Results\n\n**Host:** jobrien-vm | **Commit:** ' + sha[:8] + '\n\n\`\`\`\n' + out + '\n\`\`\`'
print(json.dumps({'body': body}))
" "$COMMIT_SHA" "$BENCH_OUT")

STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: token $TOKEN" \
  -H "Content-Type: application/json" \
  -d "$COMMENT_BODY" \
  "$GITEA_URL/api/v1/repos/$REPO/commits/$COMMIT_SHA/comments")

[[ "$STATUS" == "201" ]] \
  && echo "✓ Results posted: $GITEA_URL/$REPO/commit/$COMMIT_SHA" \
  || echo "warning: HTTP $STATUS posting comment (results printed above)" >&2
"""
```

- [ ] **Step 2: Verify the task is visible**

```bash
mise tasks | grep bench
```

Expected: both `bench` and `bench:setup` listed.

- [ ] **Step 3: Syntax-check the toml**

```bash
python3 -c "import tomllib; tomllib.load(open('mise.toml', 'rb'))"
```

Expected: no output (clean parse).

- [ ] **Step 4: Commit**

```bash
git add mise.toml
git commit -m "feat(mise): add bench task — run minibox-bench on VPS and post Gitea comment"
```
