# Ops Runtime Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Provide a production-ready runtime path for `miniboxd` via systemd with reliable socket directory setup and clear ops docs.

**Architecture:** Add a systemd unit and tmpfiles config stored in-repo under `ops/`, plus an install script that copies the binary and installs the unit safely. Document the runtime workflow in `README.md`.

**Tech Stack:** systemd, systemd-tmpfiles, bash, Rust (existing build).

---

### Task 1: Add systemd unit file

**Files:**

- Create: `ops/miniboxd.service`

**Step 1: Write the unit file**

```ini
[Unit]
Description=Minibox daemon
After=network.target
Wants=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/miniboxd
Restart=on-failure
RestartSec=2
User=root
Group=root
RuntimeDirectory=minibox
RuntimeDirectoryMode=0700
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
```

**Step 2: Quick sanity check**

Run: `systemd-analyze verify ops/miniboxd.service`
Expected: no errors (warnings ok if running in containerized env).

**Step 3: Commit**

```bash
git add ops/miniboxd.service
git commit -m "ops: add systemd unit for miniboxd"
```

### Task 2: Add tmpfiles config for /run/minibox

**Files:**

- Create: `ops/miniboxd.tmpfiles.conf`

**Step 1: Write tmpfiles config**

```ini
d /run/minibox 0700 root root -
```

**Step 2: Verify config format**

Run: `systemd-tmpfiles --cat-config | grep -F "/run/minibox"`
Expected: line for `/run/minibox` appears after install.

**Step 3: Commit**

```bash
git add ops/miniboxd.tmpfiles.conf
git commit -m "ops: add tmpfiles config for runtime socket dir"
```

### Task 3: Add install script for systemd artifacts

**Files:**

- Create: `ops/install-systemd.sh`

**Step 1: Write install script**

```bash
#!/usr/bin/env bash
set -euo pipefail

BIN_SRC="${BIN_SRC:-./target/release/miniboxd}"
BIN_DST="${BIN_DST:-/usr/local/bin/miniboxd}"
UNIT_SRC="${UNIT_SRC:-./ops/miniboxd.service}"
UNIT_DST="/etc/systemd/system/miniboxd.service"
TMPFILES_SRC="${TMPFILES_SRC:-./ops/miniboxd.tmpfiles.conf}"
TMPFILES_DST="/etc/tmpfiles.d/miniboxd.conf"

install -m 0755 "$BIN_SRC" "$BIN_DST"
install -m 0644 "$UNIT_SRC" "$UNIT_DST"
install -m 0644 "$TMPFILES_SRC" "$TMPFILES_DST"

systemctl daemon-reload
systemd-tmpfiles --create "$TMPFILES_DST"
```

**Step 2: Make executable**

Run: `chmod +x ops/install-systemd.sh`

**Step 3: Commit**

```bash
git add ops/install-systemd.sh
git commit -m "ops: add install script for systemd setup"
```

### Task 4: Document ops runtime workflow

**Files:**

- Modify: `README.md`

**Step 1: Add a new “Ops Runtime (systemd)” section**

````markdown
## Ops Runtime (systemd)

```bash
# Build
cargo build --release

# Install binary + systemd unit
sudo ./ops/install-systemd.sh

# Enable and start
sudo systemctl enable --now miniboxd

# Verify
sudo systemctl status miniboxd --no-pager
sudo /usr/local/bin/minibox ps
```
````

````

**Step 2: Commit**

```bash
git add README.md
git commit -m "docs: add ops runtime instructions"
````

### Task 5: Verify runtime flow (manual)

**Files:**

- None

**Step 1: Build**

Run: `cargo build --release`
Expected: build succeeds.

**Step 2: Install systemd artifacts**

Run: `sudo ./ops/install-systemd.sh`
Expected: no errors; daemon-reload runs.

**Step 3: Start service**

Run: `sudo systemctl enable --now miniboxd`
Expected: service active.

**Step 4: Verify socket**

Run: `sudo /usr/local/bin/minibox ps`
Expected: CLI connects to daemon and shows container list.

---

Plan complete and saved to `docs/plans/2026-03-16-ops-runtime.md`. Two execution options:

1. Subagent-Driven (this session)
2. Parallel Session (separate)

Which approach?
