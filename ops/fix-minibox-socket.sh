#!/usr/bin/env bash
set -euo pipefail

if [ ! -x target/release/miniboxd ]; then
  echo "error: target/release/miniboxd not found. Run: cargo build --release -p miniboxd" >&2
  exit 1
fi

sudo install -m 0755 target/release/miniboxd /usr/local/bin/miniboxd

sudo mkdir -p /etc/systemd/system/miniboxd.service.d
printf '%s\n' \
  '[Service]' \
  'Environment=MINIBOX_SOCKET_MODE=0660' \
  'Environment=MINIBOX_SOCKET_GROUP=minibox' \
  'RuntimeDirectoryMode=0770' \
  | sudo tee /etc/systemd/system/miniboxd.service.d/socket.conf >/dev/null

sudo systemctl daemon-reload
sudo systemctl restart miniboxd

sudo ls -ld /run/minibox
sudo ls -l /run/minibox/miniboxd.sock
