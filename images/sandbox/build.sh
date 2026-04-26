#!/usr/bin/env bash
set -euo pipefail

# Build the sandbox toolchain image and load it into minibox.
# Requires docker (or podman) and mbx on PATH.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Building minibox-sandbox image..."
docker build -t minibox-sandbox:latest "$SCRIPT_DIR"

echo "Exporting OCI tarball..."
docker save minibox-sandbox:latest -o /tmp/minibox-sandbox.tar

echo "Loading into minibox..."
sudo mbx load --name minibox-sandbox --tag latest /tmp/minibox-sandbox.tar

echo "Done. Image available as minibox-sandbox:latest"
