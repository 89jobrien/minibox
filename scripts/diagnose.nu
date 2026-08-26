#!/usr/bin/env nu
# AI-powered container failure diagnosis.
# Gathers journalctl logs, mount state, cgroup hierarchy, and runtime files
# by handing `claude -p` a Bash/Read/Glob-capable prompt.

def log-dir [] {
    $env.HOME | path join ".minibox"
}

def log-start [run_id: string, script: string, args: record] {
    let dir = (log-dir)
    mkdir $dir
    {run_id: $run_id, script: $script, args: $args, status: "running"}
    | to json -r
    | save --append ($dir | path join "agent-runs.jsonl")
}

def log-complete [run_id: string, script: string, args: record, status: string, duration_s: float, output: string] {
    let dir = (log-dir)
    {
        run_id: $run_id
        script: $script
        args: $args
        status: $status
        duration_s: ($duration_s | math round --precision 2)
        output: $output
    }
    | to json -r
    | save --append ($dir | path join "agent-runs.jsonl")
}

def build-prompt [container: string, lines: int] {
    let container_hint = if ($container | is-not-empty) {
        $"Focus on container ID: ($container)"
    } else {
        "Focus on the most recent failure."
    }

    $"Diagnose a minibox container failure. ($container_hint)

Gather evidence in this order:
1. Run `journalctl -u miniboxd -n ($lines) --no-pager` to get recent daemon logs
   \(if that fails, try `journalctl -n ($lines) --no-pager | grep -i minibox`\)
2. Run `mount | grep minibox` to check for leaked overlay mounts
3. Check cgroup state: `ls /sys/fs/cgroup/minibox.slice/miniboxd.service/ 2>/dev/null || echo 'no minibox cgroups'`
4. If a container ID is known, read:
   - `/run/minibox/containers/<id>/` for runtime state
   - `/sys/fs/cgroup/minibox.slice/miniboxd.service/<id>/` for resource limits
5. Check for common failure modes:
   - `pivot_root` EINVAL → MS_PRIVATE not set before mount namespace ops
   - overlay ENOTDIR / EINVAL → malformed lowerdir paths
   - cgroup EACCES / ENOENT → cgroup hierarchy missing or wrong path
   - clone EPERM → missing CAP_SYS_ADMIN \(check if MINIBOX_ADAPTER=gke needed\)
   - exec ENOENT → image layer extraction failed or wrong rootfs path

Report:
- **Root cause**: specific syscall/error and why it failed
- **Evidence**: the exact log lines or file contents that confirm it
- **Fix**: minimal change \(env var, config, or code pointer from CLAUDE.md\)
- **Confidence**: high / medium / low"
}

def main [
    --container: string = ""  # Container ID to focus on (optional)
    --lines: int = 200        # Daemon log lines to fetch
] {
    let prompt = (build-prompt $container $lines)
    let args = {container: $container, lines: $lines}
    let run_id = (date now | format date "%+")

    print "Diagnosing minibox failure...\n"
    log-start $run_id "diagnose" $args

    let start = (date now)
    let result = (
        do {
            ^claude -p --permission-mode acceptEdits --allowedTools "Bash,Read,Glob" $prompt
        } | complete
    )
    let elapsed = (((date now) - $start) / 1sec)

    if $result.exit_code != 0 {
        print -e $"error: diagnose failed: ($result.stderr)"
        log-complete $run_id "diagnose" $args "crash" $elapsed $result.stderr
        exit 1
    }

    print $result.stdout
    log-complete $run_id "diagnose" $args "complete" $elapsed $result.stdout
}
