#!/usr/bin/env nu
# Build the mbx-tester image inside Colima for Linux test dogfooding.
# Copies cross-compiled binaries into Colima and runs docker build there.
#
# Usage: nu scripts/build-test-image.nu [--target aarch64-unknown-linux-musl]

def main [
    --target: string = "aarch64-unknown-linux-musl"  # Cargo target triple
] {
    let target_dir = ($env.HOME | path join ".mbx" "cache" "target")
    let bin_dir = ($target_dir | path join $target "debug")
    let deps_dir = ($bin_dir | path join "deps")

    # Cross-compile everything
    print "[1/3] cross-compiling test binaries..."
    let cc = "aarch64-linux-musl-gcc"
    let env_cc = {
        CC_aarch64_unknown_linux_musl: $cc
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER: $cc
    }

    with-env $env_cc {
        ^cargo build --target $target -p miniboxd
        ^cargo build --target $target -p minibox-cli
        ^cargo test --no-run --target $target -p miniboxd --test cgroup_tests
        ^cargo test --no-run --target $target -p miniboxd --test e2e_tests
        ^cargo test --no-run --target $target -p miniboxd --test integration_tests
        ^cargo test --no-run --target $target -p miniboxd --test sandbox_tests
    }

    # Gather binary paths
    let binaries = [
        {name: "miniboxd",          path: ($bin_dir | path join "miniboxd")}
        {name: "minibox",           path: ($bin_dir | path join "minibox")}
        {name: "cgroup_tests",      path: (find-test-bin $deps_dir "cgroup_tests")}
        {name: "e2e_tests",         path: (find-test-bin $deps_dir "e2e_tests")}
        {name: "integration_tests", path: (find-test-bin $deps_dir "integration_tests")}
        {name: "sandbox_tests",     path: (find-test-bin $deps_dir "sandbox_tests")}
    ]

    # Assemble build context in a temp dir
    print "[2/3] assembling build context..."
    let ctx = (mktemp -d)
    let usr_bin = ($ctx | path join "usr" "local" "bin")
    mkdir $usr_bin

    for b in $binaries {
        cp $b.path ($usr_bin | path join $b.name)
    }

    # Write entrypoint script
    let script = '#!/bin/sh
set -e
MINIBOX_ADAPTER=native
export MINIBOX_ADAPTER

echo "=== cgroup_tests ==="
/usr/local/bin/cgroup_tests --test-threads=1 --nocapture

echo "=== integration_tests ==="
/usr/local/bin/integration_tests --test-threads=1 --ignored --nocapture

echo "=== e2e_tests ==="
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/e2e_tests --test-threads=1 --nocapture

echo "=== sandbox_tests ==="
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/sandbox_tests --test-threads=1 --ignored --nocapture

echo "=== all Linux tests passed ==="
'
    $script | save ($ctx | path join "run-tests.sh")
    ^chmod +x ($ctx | path join "run-tests.sh")

    # Write Dockerfile
    let alpine_tag = "3.21"
    let dockerfile = $"FROM alpine:($alpine_tag)
COPY usr /usr
COPY run-tests.sh /run-tests.sh
RUN chmod +x /run-tests.sh
"
    $dockerfile | save ($ctx | path join "Dockerfile")

    # Build inside Colima — tar context and pipe to docker build
    print "[3/3] building mbx-tester:latest inside Colima..."
    let tar_bytes = (
        COPYFILE_DISABLE=1 ^tar --no-xattrs -c -C $ctx . | complete
    )

    # Pipe tar to docker build via colima ssh
    (COPYFILE_DISABLE=1 ^tar --no-xattrs -c -C $ctx .) | ^colima ssh -- docker build -t mbx-tester:latest -

    rm -rf $ctx
    print "mbx-tester:latest ready in Colima"
}

# Find the compiled test binary in deps/ by prefix match
def find-test-bin [deps_dir: string, name: string] {
    let prefix = ($name | str replace --all "-" "_")
    ls $deps_dir
        | where name =~ $"($prefix)-[0-9a-f]+"
        | where { |f| ($f.name | path basename) !~ '\.' }
        | sort-by modified --reverse
        | first
        | get name
}
