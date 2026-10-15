#!/usr/bin/env bash
# Ephemeral self-hosted GHA runner entrypoint — one job, then exit.
#
# Required env (set by supervisor.sh at container start, never baked into the image):
#   RUNNER_REPO_URL     e.g. https://github.com/89jobrien/minibox
#   RUNNER_REG_TOKEN     short-lived registration token, minted fresh per container
#   RUNNER_LABELS         e.g. "self-hosted,minibox" (must match workflow runs-on)
#   RUNNER_NAME           unique per container, e.g. minibox-runner-$(hostname)
set -euo pipefail

: "${RUNNER_REPO_URL:?RUNNER_REPO_URL is required}"
: "${RUNNER_REG_TOKEN:?RUNNER_REG_TOKEN is required}"
: "${RUNNER_LABELS:=self-hosted,minibox}"
: "${RUNNER_NAME:=minibox-ephemeral-$$}"

cleanup() {
    ./config.sh remove --token "${RUNNER_REG_TOKEN}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

./config.sh \
    --url "${RUNNER_REPO_URL}" \
    --token "${RUNNER_REG_TOKEN}" \
    --name "${RUNNER_NAME}" \
    --labels "${RUNNER_LABELS}" \
    --unattended \
    --ephemeral \
    --replace

./run.sh
