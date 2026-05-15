---
name: trace-lima
description: >
  Profile miniboxd under uftrace inside the Colima VM. Run via colima ssh.
  Produces a trace directory readable with `uftrace report`.
argument-hint: "<binary-dir> <abs-trace-dir>"
---

# trace-lima

Profiles `miniboxd` using `uftrace` inside the Colima VM.

```sh
# Run inside Colima (via ssh):
colima ssh -- nu scripts/trace-lima.nu /path/to/target/release /tmp/trace-out

# View results:
colima ssh -- uftrace report -d /tmp/trace-out
```

Arguments:
- `binary-dir` — path to compiled binaries (e.g. `target/release`)
- `abs-trace-dir` — absolute path for trace output (created fresh each run)

Steps:
1. Installs `uftrace` via apt if missing
2. Starts `miniboxd` under `uftrace record`
3. Runs a pull + run smoke test
4. Kills the daemon and fixes ownership of trace output
