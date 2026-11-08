#!/usr/bin/env nu
# Run the ephemeral minibox CI runner supervisor workflow
#
# Triggers the GitHub Actions supervisor workflow
# that dispatches ephemeral minibox containers for integration/e2e jobs.
#
# GitHub CLI command:
#   gh workflow run ci-runner-supervisor --repo 89jobrien/minibox
#
# To run the supervisor locally instead:
#   env MINIBOX_ADAPTER=native MINIBOX_ALLOW_PRIVILEGED=1 \
#     RUNNER_REPO=89jobrien/minibox \
#     RUNNER_IMAGE=ghcr.io/89jobrien/minibox-ci-runner \
#     RUNNER_IMAGE_TAG=latest \
#     RUNNER_LABELS=self-hosted,minibox \
#     ops/ci-runner/supervisor.sh

echo "Minibox CI Runner Supervisor workflow trigger script"
echo "===================================================="
echo ""
echo "GitHub CLI command:"
echo "  gh workflow run ci-runner-supervisor --repo 89jobrien/minibox"
echo ""
echo "Or run the supervisor locally:"
echo "  env MINIBOX_ADAPTER=native MINIBOX_ALLOW_PRIVILEGED=1"
echo "  env RUNNER_REPO=89jobrien/minibox"
echo "  env RUNNER_IMAGE=ghcr.io/89jobrien/minibox-ci-runner"
echo "  env RUNNER_IMAGE_TAG=latest"
echo "  env RUNNER_LABELS=self-hosted,minibox"
echo "  ops/ci-runner/supervisor.sh"