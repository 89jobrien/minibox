# VPS Usage (minibox)

This guide covers running `miniboxd` and using the CLI on this VPS.

## Prerequisites

- `systemd` available
- Built binaries (`cargo build --release`)
- Root access for daemon operations

## Install + Run (systemd)

```bash
# Build
cargo build --release

# Install daemon, CLI, and systemd artifacts (includes minibox.slice)
sudo ./ops/install-systemd.sh

# Enable and start the service
sudo systemctl enable --now miniboxd

# Verify service
sudo systemctl status miniboxd --no-pager
```

## CLI Usage

```bash
# List containers
sudo /usr/local/bin/minibox ps

# Pull an image
sudo /usr/local/bin/minibox pull alpine

# Run a container
sudo /usr/local/bin/minibox run alpine -- /bin/echo "Hello from minibox!"
```

## Logs and Troubleshooting

```bash
# Follow daemon logs
sudo journalctl -u miniboxd -f
```

If the CLI fails with “No such file or directory” for `/run/minibox/miniboxd.sock`, the daemon is not running or the socket is not created yet. Start it with:

```bash
sudo systemctl enable --now miniboxd
```

## Stop / Restart

```bash
sudo systemctl stop miniboxd
sudo systemctl restart miniboxd
sudo systemctl disable --now miniboxd
```
