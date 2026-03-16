# Cgroup Debug Findings (miniboxd on VPS)

## Context
- `miniboxd` runs under systemd at `minibox.slice/miniboxd.service`.
- `MINIBOX_CGROUP_ROOT` is set to `/sys/fs/cgroup/minibox.slice/miniboxd.service/minibox`.
- `minibox pull alpine` succeeds after earlier layer extraction fixes.
- `minibox run alpine -- /bin/echo ...` fails with:
  - `error: writing cgroup file pids.max ... Permission denied`

## Evidence Collected
### Service cgroup
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

### systemd unit properties
- `Delegate=yes`
- `Slice=minibox.slice`
- `ControlGroup=/minibox.slice/miniboxd.service`
- `DelegateSubgroup` is empty

## Working Hypothesis (not yet proven)
The service cgroup has controllers available but does **not** have them enabled in
`cgroup.subtree_control`, so child cgroups (like `/minibox/...`) do not get control
of `pids`. As a result, writing `pids.max` under `/minibox/...` fails with
`Permission denied`.

## Proposed Next Investigation Step
Verify whether systemd is expected to populate `cgroup.subtree_control` for delegated
units on this host, and whether enabling `pids` for the service cgroup (or creating a
`DelegateSubgroup` or explicit sub-slice) is required by systemd policy.

## Commands Used
```
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.subtree_control
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.procs
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.events
sudo ls -la /sys/fs/cgroup/minibox.slice/miniboxd.service
sudo ls -la /sys/fs/cgroup/minibox.slice/miniboxd.service/minibox || true
sudo systemctl show -p Delegate -p Slice -p ControlGroup -p CGroupController -p DelegateSubgroup miniboxd
```
