# Ephemeral GitHub Actions runner image for the self-hosted `minibox` label.
#
# Registers with --ephemeral (one job, then exits) and is launched inside a
# --privileged minibox container by supervisor.sh — the runner needs root +
# cgroup/namespace access to run minibox's own integration/e2e suite
# (test-integration, test-e2e, test-protocol-e2e-linux, cleanup-linux in
# .github/workflows/merge.yml), which is the only reason this label exists.
FROM debian:bookworm-slim
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# hadolint ignore=DL3008
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl git jq build-essential pkg-config \
        libssl-dev sudo \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain stable --component clippy --component rustfmt
ENV PATH="/root/.cargo/bin:${PATH}"
RUN cargo install cargo-nextest --locked

ARG RUNNER_VERSION=2.321.0
WORKDIR /actions-runner
RUN curl -o actions-runner.tar.gz -L \
        "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" `# obfsck:ignore public release URL, no credentials` \
    && tar xzf actions-runner.tar.gz \
    && rm actions-runner.tar.gz \
    && ./bin/installdependencies.sh

COPY entrypoint.sh /actions-runner/entrypoint.sh
RUN chmod +x /actions-runner/entrypoint.sh

ENTRYPOINT ["/actions-runner/entrypoint.sh"]
