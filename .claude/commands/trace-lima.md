---
name: trace-lima
description: >
  Profile miniboxd under uftrace inside the Colima VM. Run via colima ssh.
  Produces a trace directory readable with `uftrace report`.
argument-hint: "<binary-dir> <abs-trace-dir>"
allowed-tools: [Bash]
---

Profile miniboxd under uftrace inside the Colima VM.

Arguments in `$ARGUMENTS`: `<binary-dir> <abs-trace-dir>` (both required).

Run these steps inside Colima via `colima ssh --`:

1. Install uftrace if absent: `sudo apt-get install -y uftrace`
2. Verify `<binary-dir>/miniboxd` exists; error if not
3. Clean and create `<abs-trace-dir>`: `rm -rf <dir> && mkdir -p <dir>`
4. Start miniboxd under uftrace in background:
   `sudo uftrace record -d <abs-trace-dir> <binary-dir>/miniboxd &`
5. `sleep 2` to let daemon settle
6. Run smoke test:
   - `sudo <binary-dir>/minibox pull alpine`
   - `sudo <binary-dir>/minibox run alpine -- /bin/true`
   - Log warnings if either fails (do not abort)
7. Stop profiler: `sudo kill -INT <daemon-pid>; sleep 1`
8. Fix ownership: `sudo chown -R $(whoami):$(whoami) <abs-trace-dir>`
9. Print: `trace saved to: <abs-trace-dir>`
10. Print: `view with: uftrace report -d <abs-trace-dir>`
