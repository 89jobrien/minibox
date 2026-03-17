#!/bin/bash
set -e

echo "=== Installing Rust ==="
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"
rustc --version
cargo --version

echo "=== Installing just ==="
cargo install just

echo "=== Installing build deps ==="
sudo apt-get update -qq
sudo apt-get install -y -qq pkg-config libssl-dev

echo "=== Checking cgroups v2 ==="
mount | grep cgroup2 || echo "WARNING: cgroup2 not mounted"
cat /proc/filesystems | grep cgroup

echo "=== Setup complete ==="
