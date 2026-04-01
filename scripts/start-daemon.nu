#!/usr/bin/env nu
# Start miniboxd with the Colima adapter, killing any existing instance first.
# Usage: nu scripts/start-daemon.nu [--adapter colima]

def main [
    --adapter: string = "colima"  # Adapter to use (colima, native, gke)
] {
    let binary = ($env.HOME | path join ".mbx" "cache" "target" "release" "miniboxd")

    if not ($binary | path exists) {
        error make {msg: $"miniboxd not found at ($binary) — run: cargo build --release -p miniboxd"}
    }

    # Kill existing daemon if running
    let pids = (do { ps | where name == "miniboxd" } | complete)
    if ($pids.exit_code == 0) {
        let running = ($pids.stdout | from nuon)
        if ($running | length) > 0 {
            print $"Stopping existing miniboxd \(($running | length) instance\(s\)\)..."
            $running | each { |p| ^kill ($p.pid | into string) }
            sleep 500ms
        }
    }

    print $"Starting miniboxd \(MINIBOX_ADAPTER=($adapter)\)..."
    with-env {
        MINIBOX_ADAPTER: $adapter
        LIMA_HOME: ($env.HOME | path join ".colima" "_lima")
    } {
        ^$binary
    }
}
