#!/usr/bin/env nu
# Bootstrap a Linux VM for minibox development.
# Installs: Rust, just, cargo-deny, cargo-audit, build deps. Checks cgroups v2.

def main [] {
    # --- Rust ---
    if (^which rustup | complete).exit_code != 0 {
        print "installing Rust..."
        ^curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | ^sh -s -- -y
        # Source cargo env
        let cargo_env = ($env.HOME | path join ".cargo" "env")
        if ($cargo_env | path exists) {
            ^bash -c $"source ($cargo_env)"
        }
    } else {
        print "Rust already installed"
        ^rustup update stable
    }

    let cargo_bin = ($env.HOME | path join ".cargo" "bin")

    # --- just ---
    if (^which just | complete).exit_code != 0 {
        print "installing just..."
        ^cargo install just
    } else {
        print "just already installed"
    }

    # --- cargo-deny ---
    if ($"($cargo_bin)/cargo-deny" | path exists) {
        print "cargo-deny already installed"
    } else {
        print "installing cargo-deny..."
        ^cargo install cargo-deny
    }

    # --- cargo-audit ---
    if ($"($cargo_bin)/cargo-audit" | path exists) {
        print "cargo-audit already installed"
    } else {
        print "installing cargo-audit..."
        ^cargo install cargo-audit
    }

    # --- build dependencies (Debian/Ubuntu) ---
    if (^which apt-get | complete).exit_code == 0 {
        print "installing build dependencies..."
        ^sudo apt-get install -y pkg-config libssl-dev
    }

    # --- cgroups v2 check ---
    print "\nchecking cgroups v2..."
    let mounts = (^mount | complete).stdout
    if ($mounts | str contains "cgroup2") {
        print "✓ cgroups v2 mounted"
    } else {
        print "✗ cgroups v2 not found — check /proc/filesystems and kernel config"
    }

    if ("/proc/filesystems" | path exists) {
        let fs = (open /proc/filesystems)
        if ($fs | str contains "cgroup2") {
            print "✓ cgroup2 filesystem supported by kernel"
        } else {
            print "✗ cgroup2 not in /proc/filesystems"
        }
    }

    print "\nsetup complete"
}
