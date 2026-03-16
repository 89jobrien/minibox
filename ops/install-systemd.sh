#!/usr/bin/env bash
set -euo pipefail

BIN_SRC="${BIN_SRC:-./target/release/miniboxd}"
BIN_DST="${BIN_DST:-/usr/local/bin/miniboxd}"
UNIT_SRC="${UNIT_SRC:-./ops/miniboxd.service}"
UNIT_DST="/etc/systemd/system/miniboxd.service"
TMPFILES_SRC="${TMPFILES_SRC:-./ops/miniboxd.tmpfiles.conf}"
TMPFILES_DST="/etc/tmpfiles.d/miniboxd.conf"

install -m 0755 "$BIN_SRC" "$BIN_DST"
install -m 0644 "$UNIT_SRC" "$UNIT_DST"
install -m 0644 "$TMPFILES_SRC" "$TMPFILES_DST"

systemctl daemon-reload
systemd-tmpfiles --create "$TMPFILES_DST"
