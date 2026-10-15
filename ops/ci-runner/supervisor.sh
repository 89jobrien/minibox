#!/usr/bin/env bash
# Runs one ephemeral GHA runner container at a time under minibox, forever.
#
# Each iteration mints a fresh registration token (they expire in ~1h, so never
# bake one into the image or reuse across containers), then blocks on `mbx run`
# until the runner picks up exactly one job and exits. Requires `gh` to be
# authenticated with repo-admin scope and `mbx` on PATH.
#
# Install as a systemd service (Type=simple, Restart=on-failure) rather than a
# cron loop, so a crashed container is retried without a gap longer than
# RestartSec.
set -euo pipefail

REPO="${RUNNER_REPO:-89jobrien/minibox}"
IMAGE="${RUNNER_IMAGE:-ghcr.io/89jobrien/minibox-ci-runner}"
IMAGE_TAG="${RUNNER_IMAGE_TAG:-latest}"
LABELS="${RUNNER_LABELS:-self-hosted,minibox}"

while true; do
    TOKEN=$(gh api "repos/${REPO}/actions/runners/registration-token" --jq .token)
    NAME="minibox-ephemeral-$(date +%s)"

    echo "supervisor: starting ${NAME}"
    mbx run --privileged --rm \
        --tag "${IMAGE_TAG}" \
        --env "RUNNER_REPO_URL=https://github.com/${REPO}" \
        --env "RUNNER_REG_TOKEN=${TOKEN}" \
        --env "RUNNER_LABELS=${LABELS}" \
        --env "RUNNER_NAME=${NAME}" \
        "${IMAGE}"

    echo "supervisor: ${NAME} exited, restarting"
done
