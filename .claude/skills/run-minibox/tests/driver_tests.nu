#!/usr/bin/env nu

def check [condition: bool, message: string] {
    if not $condition {
        error make {msg: $message}
    }
}

def invoke [driver: path, vars: record, command: string, ...args: string] {
    let quoted = ($args | each {|arg| $"\"($arg)\"" } | str join " ")
    let code = $"source \"($driver)\"; ($command) ($quoted)"
    with-env $vars {
        do { ^nu -c $code } | complete
    }
}

def main [] {
    let skill = ($env.FILE_PWD | path dirname)
    let driver = ($skill | path join "driver.nu")
    let temp = (^mktemp -d | str trim)
    let repo = ($temp | path join "repo")
    let run_dir = ($temp | path join "run")
    let record = ($temp | path join "record")
    mkdir $repo $run_dir

    let cargo = ($temp | path join "cargo")
    $'#!/bin/sh
printf "%s\\n" "$*" > "($record)"
' | save $cargo
    ^chmod +x $cargo

    let cli = ($temp | path join "mbx")
    $'#!/bin/sh
printf "socket=%s\\nargs=%s\\n" "$MINIBOX_SOCKET_PATH" "$*" > "($record)"
' | save $cli
    ^chmod +x $cli

    let daemon = ($temp | path join "miniboxd")
    let observed_pid = ($temp | path join "observed-pid")
    $'#!/bin/sh
printf "%s" "$$" > "($observed_pid)"
touch "$MINIBOX_SOCKET_PATH"
while :; do sleep 1; done
' | save $daemon
    ^chmod +x $daemon

    let vars = {
        MINIBOX_RUN_REPO: $repo
        MINIBOX_RUN_DIR: $run_dir
        MINIBOX_RUN_CARGO_BIN: $cargo
        MINIBOX_RUN_CLI_BIN: $cli
        MINIBOX_RUN_DAEMON_BIN: $daemon
        MINIBOX_RUN_START_TIMEOUT_MS: "2000"
    }

    let build = (invoke $driver $vars "build")
    check ($build.exit_code == 0) $"build failed: ($build.stderr)"
    check ((open --raw $record | str trim) == "build --release -p miniboxd -p minibox-cli") "build used the wrong Cargo package"

    let forwarded = (invoke $driver $vars "run-cmd" "alpine:latest" "printf" "hello world")
    check ($forwarded.exit_code == 0) $"run-cmd failed: ($forwarded.stderr)"
    let invocation = (open --raw $record)
    check ($invocation | str contains $"socket=($run_dir)/miniboxd.sock") "socket env was not forwarded"
    check ($invocation | str contains "args=run alpine:latest -- printf hello world") "arguments were not forwarded"

    let missing_env = ($vars | upsert MINIBOX_RUN_DAEMON_BIN ($temp | path join "missing"))
    let missing = (invoke $driver $missing_env "start")
    check ($missing.exit_code != 0) "missing daemon binary did not fail"
    check (not (($run_dir | path join "daemon.pid") | path exists)) "missing binary left a pid file"

    "1" | save ($run_dir | path join "daemon.pid")
    "stale" | save ($run_dir | path join "miniboxd.sock")
    let started = (invoke $driver $vars "start")
    check ($started.exit_code == 0) $"start failed: ($started.stderr)"
    let stored_pid = (open --raw ($run_dir | path join "daemon.pid") | str trim)
    let actual_pid = (open --raw $observed_pid | str trim)
    check ($stored_pid == $actual_pid) $"captured pid ($stored_pid) != daemon pid ($actual_pid)"
    check (($run_dir | path join "miniboxd.sock") | path exists) "start succeeded without expected socket"

    let teardown = (invoke $driver $vars "teardown")
    check ($teardown.exit_code == 0) $"teardown failed: ($teardown.stderr)"
    check (not ($run_dir | path exists)) "teardown left the run directory"

    mkdir $run_dir
    "1" | save ($run_dir | path join "daemon.pid")
    let stale = (invoke $driver $vars "teardown")
    check ($stale.exit_code == 0) $"stale pid teardown failed: ($stale.stderr)"
    check (not ($run_dir | path exists)) "stale pid teardown left the run directory"

    let failing_daemon = ($temp | path join "failing-miniboxd")
    $'#!/bin/sh
exit 23
' | save $failing_daemon
    ^chmod +x $failing_daemon
    let failure_env = ($vars | upsert MINIBOX_RUN_DAEMON_BIN $failing_daemon)
    let failed_start = (invoke $driver $failure_env "start")
    check ($failed_start.exit_code != 0) "daemon start failure was not detected"
    check (not (($run_dir | path join "daemon.pid") | path exists)) "failed start left a pid file"

    rm -rf $temp
    print "driver tests passed"
}
