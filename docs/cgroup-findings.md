# Cgroup Debug Findings (miniboxd on VPS)

## Context
- `miniboxd` runs under systemd at `minibox.slice/miniboxd.service`.
- `minibox pull alpine` succeeds after earlier layer extraction fixes.
- `minibox run alpine -- /bin/true` fails with:
  - `error: writing cgroup file pids.max ... Permission denied`

## Evidence Collected
### Service cgroup (before DelegateSubgroup)
- `cgroup.controllers`:
  - `cpuset cpu io memory pids`
- `cgroup.subtree_control`:
  - empty (no controllers delegated to children)
- `cgroup.type`:
  - `domain`
- `cgroup.procs`:
  - contains the miniboxd PID
- Service cgroup directory has `minibox/` child present.

### minibox child cgroup
- `minibox/` exists under the service cgroup and already contains a per-container child
  (`cf0235011ade432a`).
- `minibox/cgroup.subtree_control`:
  - empty (no controllers enabled for its children)

### systemd unit properties (before DelegateSubgroup)
- `Delegate=yes`
- `Slice=minibox.slice`
- `ControlGroup=/minibox.slice/miniboxd.service`
- `DelegateSubgroup` is empty

## Updated Findings (after DelegateSubgroup)
- `DelegateSubgroup=yes` creates a delegated subgroup at:
  - `/sys/fs/cgroup/minibox.slice/miniboxd.service/yes`
- The running daemon is inside that subgroup:
  - `CGroup: /minibox.slice/miniboxd.service/yes`
- `MINIBOX_CGROUP_ROOT` was updated to:
  - `/sys/fs/cgroup/minibox.slice/miniboxd.service/yes/minibox`
- Attempts to enable `pids` controllers in the delegated subgroup fail:
  - `echo "+pids" > .../yes/cgroup.subtree_control` → `I/O error`
  - `echo "+pids" > .../yes/minibox/cgroup.subtree_control` → `I/O error`
- `minibox run` still fails writing `pids.max` under:
  - `/sys/fs/cgroup/minibox.slice/miniboxd.service/yes/minibox/<id>/pids.max`

## Root Cause (current best understanding)
In cgroup v2, a cgroup that contains processes cannot enable controllers for
its children. With `DelegateSubgroup=yes`, systemd places `miniboxd` inside
`/miniboxd.service/yes`, which means `/yes` contains a process. As a result,
`/yes/cgroup.subtree_control` cannot be modified (I/O error), so controllers
cannot be delegated to child cgroups. This blocks writes to `pids.max` under
`/yes/minibox/<id>`.

## Proposed Next Fix
Stop using the delegated subgroup as the parent for containers. Instead:
- Remove `DelegateSubgroup=yes`
- Keep `miniboxd` in `/miniboxd.service`
- Enable controllers on `/miniboxd.service` before creating `/miniboxd.service/minibox`
- Set `MINIBOX_CGROUP_ROOT` back to:
  - `/sys/fs/cgroup/minibox.slice/miniboxd.service/minibox`

This can be enforced via `ExecStartPre` in the systemd unit.

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
