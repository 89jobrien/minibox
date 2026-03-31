# Cgroup Debug Findings (miniboxd on VPS)

## Context

- `miniboxd` runs under systemd at `minibox.slice/miniboxd.service`.
- `minibox pull alpine` succeeds after earlier layer extraction fixes.
- `minibox run alpine -- /bin/true` was failing with:
  - `error: writing cgroup file pids.max ... Permission denied`

## Root Cause

cgroup v2 enforces a "no internal processes" rule: a cgroup that contains
processes cannot enable `subtree_control` for its children. miniboxd was
living in the same cgroup where it tried to create container children, so
the kernel rejected writes to `pids.max`, `memory.max`, etc.

Two earlier attempts failed to solve this:

1. **Enabling controllers on the service cgroup** -- worked for the
   immediate child, but the `minibox/` subdirectory had `cgroup.type =
domain invalid` and couldn't delegate further (I/O error).
2. **`DelegateSubgroup=yes`** -- `DelegateSubgroup` takes a cgroup name
   string, not a boolean. Setting it to `yes` created a literal `/yes`
   subgroup. The daemon lived in `/yes`, so the same no-internal-processes
   constraint applied and `subtree_control` writes to `/yes` failed.

## Fix (commit 8e97d3f)

The fix uses a **supervisor leaf cgroup** pattern -- the standard approach
used by containerd, slurm, and recommended by the systemd cgroup delegation
documentation.

### Approach: supervisor leaf + sibling containers

The daemon process is isolated in a leaf cgroup (`supervisor/`). Container
cgroups are created as siblings, not children. The service cgroup becomes a
pure inner node that can freely enable `subtree_control`.

```
/minibox.slice/miniboxd.service/       <- inner node, no processes
    cgroup.subtree_control             <- +pids +memory +cpu +io
    supervisor/                        <- daemon PID lives here (leaf)
    {container_id}/                    <- container cgroup (leaf)
```

### Changes made

**`ops/miniboxd.service`:**

- `DelegateSubgroup=supervisor` -- systemd places the daemon in
  `/miniboxd.service/supervisor/` automatically (requires systemd >= 254).
- `MINIBOX_CGROUP_ROOT=/sys/fs/cgroup/minibox.slice/miniboxd.service` --
  containers are created directly under the service cgroup.

**`crates/miniboxd/src/main.rs`:**

- Added `migrate_to_supervisor_cgroup()` runtime fallback. On startup the
  daemon reads `/proc/self/cgroup`, creates a `supervisor/` child, and moves
  its own PID there. No-op if systemd `DelegateSubgroup` already handled it.

**`crates/mbx/src/container/cgroups.rs`:**

- `enable_subtree_controllers()` writes `+pids +memory +cpu +io` to
  `cgroup.subtree_control` at the cgroup root before creating container
  children. Idempotent, non-fatal if a controller is unavailable.

## Evidence Collected (historical)

### Service cgroup (before DelegateSubgroup)

- `cgroup.controllers`: `cpuset cpu io memory pids`
- `cgroup.subtree_control`: empty (no controllers delegated to children)
- `cgroup.type`: `domain`
- `cgroup.procs`: contains the miniboxd PID
- Service cgroup directory has `minibox/` child present.

### minibox child cgroup

- `minibox/` exists under the service cgroup and already contains a per-container child
  (`cf0235011ade432a`).
- `minibox/cgroup.subtree_control`: empty (no controllers enabled for its children)

### systemd unit properties (before DelegateSubgroup)

- `Delegate=yes`
- `Slice=minibox.slice`
- `ControlGroup=/minibox.slice/miniboxd.service`
- `DelegateSubgroup` is empty

### After DelegateSubgroup=yes (broken)

- `DelegateSubgroup=yes` creates a delegated subgroup at:
  - `/sys/fs/cgroup/minibox.slice/miniboxd.service/yes`
- The running daemon is inside that subgroup:
  - `CGroup: /minibox.slice/miniboxd.service/yes`
- Attempts to enable `pids` controllers in the delegated subgroup fail:
  - `echo "+pids" > .../yes/cgroup.subtree_control` -> `I/O error`
  - Explanation: `/yes` contains a process, so controllers cannot be enabled
    for its children in cgroup v2.

## Final Fix Applied (Works)

The following upstream changes resolved the issue:

- systemd unit:
  - `DelegateSubgroup=supervisor`
  - `MINIBOX_CGROUP_ROOT=/sys/fs/cgroup/minibox.slice/miniboxd.service`
- daemon:
  - `miniboxd` migrates itself into a `supervisor` leaf cgroup on startup
    (creates `/sys/fs/cgroup/<current>/supervisor` and writes its PID).

Result after rebuild + reinstall:

- `~/.cargo/bin/cargo build --release`
- `sudo ./ops/install-systemd.sh`
- `sudo systemctl daemon-reload`
- `sudo systemctl restart miniboxd`
- `sudo /usr/local/bin/minibox run alpine -- /bin/true`
  - Succeeds and returns a container ID.

## Detailed Try Log (chronological)

- Enable daemon + run:
  - `sudo systemctl enable --now miniboxd`
  - `sudo /usr/local/bin/minibox run alpine -- /bin/true`
  - Fails with `pids.max: Permission denied`
- Inspect service cgroup:
  - `/sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.subtree_control` empty
- Attempt to enable controllers on service cgroup:
  - `echo "+pids" > .../cgroup.subtree_control` succeeded
  - Still fails writing `pids.max` under `/minibox/...`
- Inspect `/minibox`:
  - `cgroup.type` = `domain invalid`
  - `cgroup.procs` empty, but `echo "+pids"` to `/minibox/cgroup.subtree_control`
    returns `I/O error`
- Enable `DelegateSubgroup=yes`:
  - systemd creates `/miniboxd.service/yes`
  - `miniboxd` runs inside `/yes`
  - `MINIBOX_CGROUP_ROOT` set to `/miniboxd.service/yes/minibox`
  - `minibox run` still fails writing `pids.max`
  - `echo "+pids"` to `/yes/cgroup.subtree_control` returns `I/O error`
  - `echo "+pids"` to `/yes/minibox/cgroup.subtree_control` returns `I/O error`
  - Explanation: `/yes` contains a process, so controllers cannot be enabled
    for its children in cgroup v2.
- Pull new upstream changes:
  - `DelegateSubgroup=supervisor` in systemd unit
  - `miniboxd` self-migrates into `supervisor` leaf cgroup
  - `MINIBOX_CGROUP_ROOT=/sys/fs/cgroup/minibox.slice/miniboxd.service`
- Rebuild and reinstall:
  - `~/.cargo/bin/cargo build --release`
  - `sudo ./ops/install-systemd.sh`
  - `sudo systemctl daemon-reload`
  - `sudo systemctl restart miniboxd`
- Verify run succeeds:
  - `sudo /usr/local/bin/minibox run alpine -- /bin/true`
  - Returns container ID `766500c88f0347c1`

## Verification Commands

```bash
# Check daemon is in supervisor leaf
sudo systemctl status miniboxd --no-pager
# Should show: CGroup: /minibox.slice/miniboxd.service/supervisor

# Verify subtree_control is populated
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.subtree_control
# Should show: cpu io memory pids

# Test container run
sudo /usr/local/bin/minibox run alpine -- /bin/true
```

## Commands Used

```
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.subtree_control
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.procs
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.events
sudo ls -la /sys/fs/cgroup/minibox.slice/miniboxd.service
sudo ls -la /sys/fs/cgroup/minibox.slice/miniboxd.service/minibox || true
sudo systemctl show -p Delegate -p Slice -p ControlGroup -p CGroupController -p DelegateSubgroup miniboxd
sudo sh -c 'echo "+pids" > /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.subtree_control'
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/minibox/cgroup.type
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/minibox/cgroup.procs
sudo systemctl cat miniboxd
sudo systemctl show -p Environment miniboxd
sudo sh -c 'tr "\0" "\n" < /proc/<PID>/environ | grep MINIBOX_CGROUP_ROOT'
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/yes/cgroup.subtree_control
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/yes/minibox/cgroup.subtree_control
sudo find /sys/fs/cgroup/minibox.slice/miniboxd.service/yes -maxdepth 2 -type d
~/.cargo/bin/cargo build --release
sudo ./ops/install-systemd.sh
sudo systemctl daemon-reload
sudo systemctl restart miniboxd
sudo /usr/local/bin/minibox run alpine -- /bin/true
```
