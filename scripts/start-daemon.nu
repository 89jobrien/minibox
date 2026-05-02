#!/usr/bin/env nu
# Thin wrapper: resolves the miniboxd binary and delegates to it.
#
# Restart logic is built into miniboxd --restart.
# Use --adapter (or MINIBOX_ADAPTER) to select the adapter suite.
# Run `mbx doctor` to see which adapters are compiled into this build.
#
# Usage: nu scripts/start-daemon.nu [--adapter colima]

def main [
    --adapter: string = ""  # Adapter to use (colima, smolvm, krun, native, gke). Default: auto.
] {
    let binary = ($env.HOME | path join ".minibox" "cache" "target" "release" "miniboxd")

    if not ($binary | path exists) {
        error make {msg: $"miniboxd not found at ($binary) — run: cargo build --release -p miniboxd"}
    }

    let adapter_display = if ($adapter | is-empty) { "auto" } else { $adapter }
    print $"Starting miniboxd \(MINIBOX_ADAPTER=($adapter_display)\) with --restart..."

    let env_overrides = if ($adapter | is-empty) {
        {}
    } else {
        {MINIBOX_ADAPTER: $adapter}
    }

    with-env $env_overrides {
        ^$binary --restart
    }
}
