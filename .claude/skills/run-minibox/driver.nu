#!/usr/bin/env nu
# Driver for exercising a real miniboxd + mbx lifecycle in isolation from any
# system-level minibox daemon. Verified on macOS (Darwin) with the smolvm
# adapter. Each subcommand is independent; run `start` first.
#
# Usage:
#   nu driver.nu build                 # cargo build --release -p miniboxd -p minibox-cli
#   nu driver.nu start                 # boot an isolated miniboxd, print PID
#   nu driver.nu status                # mbx doctor + mbx ps against the isolated daemon
#   nu driver.nu smoke                 # pull alpine, run one command, show output
#   nu driver.nu ps                    # mbx ps
#   nu driver.nu logs <name-or-id>
#   nu driver.nu stop <name-or-id>
#   nu driver.nu rm <name-or-id>
#   nu driver.nu teardown              # kill the isolated daemon, remove run dir

const DEFAULT_RUN_DIR = "/tmp/mbx-run"
const DEFAULT_REPO = "/Users/joe/dev/minibox"

def setting [name: string, fallback: string] {
    $env | get -o $name | default $fallback
}

def repo [] { setting "MINIBOX_RUN_REPO" $DEFAULT_REPO }
def run-dir [] { setting "MINIBOX_RUN_DIR" $DEFAULT_RUN_DIR }
def cargo-bin [] { setting "MINIBOX_RUN_CARGO_BIN" "cargo" }
def mbx-bin [] { setting "MINIBOX_RUN_CLI_BIN" $"(repo)/target/release/mbx" }
def miniboxd-bin [] { setting "MINIBOX_RUN_DAEMON_BIN" $"(repo)/target/release/miniboxd" }

def sock [] { (run-dir) | path join "miniboxd.sock" }
def state-dir [] { (run-dir) | path join "state" }
def log-file [] { (run-dir) | path join "daemon.log" }
def pid-file [] { (run-dir) | path join "daemon.pid" }

def process-command [pid: int] {
    let result = (do { ^ps -p $pid -o command= } | complete)
    if $result.exit_code == 0 { $result.stdout | str trim } else { null }
}

def pid-from-file [] {
    if not (pid-file | path exists) {
        return null
    }
    try { open --raw (pid-file) | str trim | into int } catch { null }
}

def ensure-binary [binary: path, label: string] {
    if not ($binary | path exists) {
        error make {msg: $"($label) binary not found: ($binary)"}
    }
}

def process-is-owned [command: string, binary: path] {
    let expected = ($binary | into string)
    $command | split row --regex '\s+' | any {|token| $token == $expected }
}

def stop-owned-process [pid: int, binary: path] {
    let command = (process-command $pid)
    if $command == null {
        return false
    }
    if not (process-is-owned $command $binary) {
        print $"refusing to kill stale pid ($pid): process is not ($binary)"
        return false
    }
    try { kill $pid }
    sleep 100ms
    if (process-command $pid) != null {
        try { kill --force $pid }
    }
    true
}

export def build [] {
    cd (repo)
    ^(cargo-bin) build --release -p miniboxd -p minibox-cli
}

export def start [] {
    let bin = (miniboxd-bin)
    ensure-binary $bin "miniboxd"

    let previous_pid = (pid-from-file)
    if $previous_pid != null {
        let previous_command = (process-command $previous_pid)
        if $previous_command != null and (process-is-owned $previous_command $bin) {
            error make {msg: $"miniboxd is already running with pid ($previous_pid)"}
        }
        rm -f (pid-file)
    }

    mkdir (run-dir)
    mkdir (state-dir)
    rm -f (sock)
    cd (repo)
    $env.MINIBOX_DATA_DIR = (state-dir)
    $env.MINIBOX_SOCKET_PATH = (sock)
    let lf = (log-file)
    let launched = (do { ^sh -c 'nohup "$1" >"$2" 2>&1 & echo $!' sh $bin $lf } | complete)
    if $launched.exit_code != 0 {
        error make {msg: $"failed to launch miniboxd: ($launched.stderr | str trim)"}
    }
    let daemon_pid = (try { $launched.stdout | str trim | into int } catch {
        error make {msg: $"launcher returned an invalid miniboxd pid: ($launched.stdout | str trim)"}
    })

    let timeout_ms = (setting "MINIBOX_RUN_START_TIMEOUT_MS" "5000" | into int)
    let attempts = (($timeout_ms / 50) | math ceil | into int)
    mut ready = false
    for _ in 0..$attempts {
        if (sock | path exists) {
            $ready = true
            break
        }
        sleep 50ms
    }
    if not $ready {
        stop-owned-process $daemon_pid $bin | ignore
        rm -f (pid-file) (sock)
        error make {msg: $"miniboxd failed to create expected socket (sock); see ($lf)"}
    }

    $daemon_pid | save -f (pid-file)
    print $"miniboxd started, pid=($daemon_pid), socket=(sock)"
}

export def status [] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    print "--- mbx doctor (adapter capability report) ---"
    ^(mbx-bin) doctor
    print "--- mbx ps ---"
    ^(mbx-bin) ps
}

export def "run-cmd" [image: string, ...cmd: string] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) run $image -- ...$cmd
}

export def smoke [] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    print "--- pull alpine:latest ---"
    ^(mbx-bin) pull alpine:latest
    print "--- run: echo through the VM ---"
    ^(mbx-bin) run --name run-skill-smoke alpine:latest -- echo hello-from-mbx
    print "--- ps (should be empty: run is ephemeral, container is auto-removed on exit) ---"
    ^(mbx-bin) ps
}

export def ps-list [] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) ps
}

export def logs [target: string] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) logs $target
}

export def stop [target: string] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) stop $target
}

export def rm-container [target: string] {
    ensure-binary (mbx-bin) "mbx"
    cd (repo)
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) rm $target
}

export def teardown [] {
    let daemon_pid = (pid-from-file)
    if $daemon_pid != null {
        if not (stop-owned-process $daemon_pid (miniboxd-bin)) {
            print $"pid ($daemon_pid) already gone or stale"
        }
    }
    rm -rf (run-dir)
    print "torn down isolated miniboxd + state dir"
}
