#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-v0.1.0}"
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m | sed 's/x86_64/amd64/g; s/aarch64/arm64/g')

echo "Installing miniboxctl $VERSION for $OS/$ARCH..."

RELEASE_URL="https://github.com/dagu-org/miniboxctl/releases/download/${VERSION}/miniboxctl-${OS}-${ARCH}"
curl -fSL "$RELEASE_URL" -o /tmp/miniboxctl || {
    echo "Failed to download from $RELEASE_URL" >&2
    exit 1
}

chmod +x /tmp/miniboxctl

if [[ -d /etc/systemd/system && $EUID -eq 0 ]]; then
    echo "Installing as systemd service..."
    mv /tmp/miniboxctl /usr/local/bin/
    cat > /etc/systemd/system/miniboxctl.service <<'EOF'
[Unit]
Description=Minibox Controller
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/miniboxctl --listen localhost:9999
# Override with --listen 0.0.0.0:9999 for network-accessible deployments (add auth first)
Restart=on-failure
RestartSec=5s
Environment=MINIBOX_SOCKET_PATH=/run/minibox/miniboxd.sock

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable miniboxctl
    systemctl start miniboxctl
    echo "Installed and started. Check status: systemctl status miniboxctl"
else
    echo "Installing to ~/.local/bin..."
    mkdir -p ~/.local/bin
    mv /tmp/miniboxctl ~/.local/bin/
    echo "Installed to ~/.local/bin/miniboxctl"
    echo "Start manually: ~/.local/bin/miniboxctl --listen localhost:9999"
fi
