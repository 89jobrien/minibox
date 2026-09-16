#!/usr/bin/env nu
# Driver for exercising a real miniboxd + mbx lifecycle in isolation from any
# system-level minibox daemon. Verified on macOS (Darwin) with the smolvm
# adapter. Each subcommand is independent; run `start` first.
#
# Usage:
#   nu driver.nu build                 # cargo build --release -p miniboxd -p mbx
#   nu driver.nu start                 # boot an isolated miniboxd, print PID
#   nu driver.nu status                # mbx doctor + mbx ps against the isolated daemon
#   nu driver.nu smoke                 # pull alpine, run one command, show output
#   nu driver.nu ps                    # mbx ps
#   nu driver.nu logs <name-or-id>
#   nu driver.nu stop <name-or-id>
#   nu driver.nu rm <name-or-id>
#   nu driver.nu teardown              # kill the isolated daemon, remove run dir

const RUN_DIR = "/tmp/mbx-run"
const REPO = "/Users/joe/dev/minibox"

def mbx-bin [] { $"($REPO)/target/release/mbx" }
def miniboxd-bin [] { $"($REPO)/target/release/miniboxd" }

def sock [] { $"($RUN_DIR)/miniboxd.sock" }
def state-dir [] { $"($RUN_DIR)/state" }
def log-file [] { $"($RUN_DIR)/daemon.log" }
def pid-file [] { $"($RUN_DIR)/daemon.pid" }

export def build [] {
    cd $REPO
    cargo build --release -p miniboxd -p mbx
}

export def start [] {
    mkdir (state-dir)
    cd $REPO
    $env.MINIBOX_STATE_DIR = (state-dir)
    $env.MINIBOX_SOCKET_PATH = (sock)
    let bin = (miniboxd-bin)
    let lf = (log-file)
    # job spawn / trailing `&` both die when this `nu -c` process exits — use
    # a detached shell (setsid, falls back to plain `sh -c ... &` on macOS
    # where setsid isn't available) so the daemon survives past this call.
    ^sh -c $"nohup '($bin)' > '($lf)' 2>&1 &"
    sleep 1sec
    let found = (ps | where name =~ "miniboxd" | sort-by pid | last)
    $found.pid | save -f (pid-file)
    print $"miniboxd started, pid=($found.pid), socket=(sock)"
}

export def status [] {
    cd $REPO
    $env.MINIBOX_SOCKET_PATH = (sock)
    print "--- mbx doctor (adapter capability report) ---"
    ^(mbx-bin) doctor
    print "--- mbx ps ---"
    ^(mbx-bin) ps
}

export def "run-cmd" [image: string, ...cmd: string] {
    cd $REPO
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) run $image -- ...$cmd
}

export def smoke [] {
    cd $REPO
    $env.MINIBOX_SOCKET_PATH = (sock)
    print "--- pull alpine:latest ---"
    ^(mbx-bin) pull alpine:latest
    print "--- run: echo through the VM ---"
    ^(mbx-bin) run --name run-skill-smoke alpine:latest -- echo hello-from-mbx
    print "--- ps (should be empty: run is ephemeral, container is auto-removed on exit) ---"
    ^(mbx-bin) ps
}

export def ps-list [] {
    cd $REPO
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) ps
}

export def logs [target: string] {
    cd $REPO
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) logs $target
}

export def stop [target: string] {
    cd $REPO
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) stop $target
}

export def rm-container [target: string] {
    cd $REPO
    $env.MINIBOX_SOCKET_PATH = (sock)
    ^(mbx-bin) rm $target
}

export def teardown [] {
    if (pid-file | path exists) {
        let p = (open (pid-file) | into int)
        try { kill --force $p } catch { print $"pid ($p) already gone" }
    }
    rm -r -f $RUN_DIR
    print "torn down isolated miniboxd + state dir"
}
