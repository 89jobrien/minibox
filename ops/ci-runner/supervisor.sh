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
ITERATION=0

token_expiry_warning() {
    # Registration tokens expire in ~1 hour (3600s). If we've been running
    # for a while, warn if the token might be close to expiry.
    local elapsed=$1
    if ((elapsed > 2700)); then
        echo "supervisor: WARNING: iteration has run for ${elapsed}s (~2700s = 45min),"
        echo "  token may be close to 3600s expiry. Next iteration will mint a fresh token."
    fi
}

while true; do
    ITERATION=$((ITERATION + 1))
    TOKEN=$(gh api "repos/${REPO}/actions/runners/registration-token" --jq .token)
    NAME="minibox-ephemeral-$(date +%s)"

    ITERATION_SECONDS=$SECONDS
    token_expiry_warning $ITERATION_SECONDS

    echo "supervisor: iteration ##{ITERATION} starting ${NAME}"
    mbx run --privileged --rm \
        --tag "${IMAGE_TAG}" \
        --env "RUNNER_REPO_URL=https://github.com/${REPO}" \
        --env "RUNNER_REG_TOKEN=${TOKEN}" \
        --env "RUNNER_LABELS=${LABELS}" \
        --env "RUNNER_NAME=${NAME}" \
        "${IMAGE}"

    EXIT_CODE=$?
    echo "supervisor: iteration ##{ITERATION} ${NAME} exited (exit code: ${EXIT_CODE})"

    # If mbx run exited with an error, check if it might be a token issue
    if ((EXIT_CODE != 0)); then
        echo "supervisor: iteration ##{ITERATION} job failed — will mint fresh token on next loop"
    fi

    # Brief pause before minting next token to avoid rate flooding
    sleep 1
done
