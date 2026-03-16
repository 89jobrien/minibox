#!/usr/bin/env bash
set -euo pipefail

BIN_SRC="${BIN_SRC:-./target/release/miniboxd}"
BIN_DST="${BIN_DST:-/usr/local/bin/miniboxd}"
CLI_SRC="${CLI_SRC:-./target/release/minibox}"
CLI_DST="${CLI_DST:-/usr/local/bin/minibox}"
UNIT_SRC="${UNIT_SRC:-./ops/miniboxd.service}"
UNIT_DST="/etc/systemd/system/miniboxd.service"
SLICE_SRC="${SLICE_SRC:-./ops/minibox.slice}"
SLICE_DST="/etc/systemd/system/minibox.slice"
TMPFILES_SRC="${TMPFILES_SRC:-./ops/miniboxd.tmpfiles.conf}"
TMPFILES_DST="/etc/tmpfiles.d/miniboxd.conf"

install -m 0755 "$BIN_SRC" "$BIN_DST"
install -m 0755 "$CLI_SRC" "$CLI_DST"
install -m 0644 "$UNIT_SRC" "$UNIT_DST"
install -m 0644 "$SLICE_SRC" "$SLICE_DST"
install -m 0644 "$TMPFILES_SRC" "$TMPFILES_DST"

systemctl daemon-reload
systemd-tmpfiles --create "$TMPFILES_DST"
