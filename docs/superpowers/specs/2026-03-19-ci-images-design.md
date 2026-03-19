# CI Images Design

**Date:** 2026-03-19
**Status:** Draft

## Overview

Pre-baked OCI images containing the Rust toolchain and CI tooling. Images are published to `ghcr.io` via a scheduled GHA workflow and pulled locally to `~/.mbx/cache/`. The goal is zero cold-start compilation in CI — all toolchain and dependency layers are pre-built into the image.

---

## Image Inventory

### `ghcr.io/<org>/minibox-rust-ci:stable`

The primary CI image. Used for fmt, clippy, nextest, coverage, and benchmarks.

**Contents:**

| Layer        | Contents                                                                                                  |
| ------------ | --------------------------------------------------------------------------------------------------------- |
| Base         | `debian:slim`                                                                                             |
| Toolchain    | Rust stable (via rustup), edition 2024, `rustfmt` + `clippy` components                                   |
| Cargo tools  | `cargo-nextest`, `cargo-llvm-cov`, `samply`, `cargo-deny`, `cargo-audit`                                  |
| System tools | `lld` (fast linker), `git`, `curl`, `jq`                                                                  |
| Env          | `CARGO_INCREMENTAL=0`, `CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse`, `RUSTFLAGS=-C link-arg=-fuse-ld=lld` |

**Not included:** workspace source, workspace deps. The image provides tooling only — source is bind-mounted at runtime (Phase 2+). This keeps the image small and avoids rebuilding on every source change.

**Phase 0 note:** In Phase 0, this image is pulled and cached only — it is not yet integrated into CI workflows. Phase 1 validates stdout piping with a smoke test. Phase 2 integrates the image into local hooks via bind mounts. See containerized-ci-execution-design.md for the full phased rollout.

### `ghcr.io/<org>/minibox-rust-ci:nightly` (future)

Same as `stable` but with the nightly toolchain. Used for nightly benchmarks and experimental features. Not in Phase 0 scope.

---

## Dockerfile

```dockerfile
FROM debian:bookworm-slim

# System deps + lld
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl git jq ca-certificates lld \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y \
    --default-toolchain stable \
    --component rustfmt clippy
ENV PATH="/root/.cargo/bin:${PATH}"

# Cargo tools (pre-built binaries where possible)
RUN cargo install cargo-nextest --locked \
    && cargo install cargo-llvm-cov --locked \
    && cargo install cargo-deny --locked \
    && cargo install cargo-audit --locked \
    && cargo install samply --locked

# CI-optimised env
ENV CARGO_INCREMENTAL=0
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld"

WORKDIR /workspace
```

**Build time note:** `cargo install` for the tools takes ~5-10 min on first build. This cost is paid once during image publish, never during CI runs.

---

## Build + Publish Workflow

### `image-ci.yml` — GHA workflow

```yaml
name: Publish CI Image

on:
  schedule:
    - cron: '0 2 * * 0'   # Weekly, Sunday 02:00 UTC
  workflow_dispatch:

permissions:
  contents: read
  packages: write

jobs:
  build-push:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Log in to GHCR
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Docker meta
        id: meta
        uses: docker/metadata-action@v5
        with:
          images: ghcr.io/${{ github.repository_owner }}/minibox-rust-ci
          tags: |
            type=raw,value=stable
            type=raw,value={{date 'YYYY-MM-DD'}}
            type=sha,prefix=sha-,format=short

      - name: Set up QEMU
        uses: docker/setup-qemu-action@v3

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Build and push
        uses: docker/build-push-action@v5
        with:
          context: .
          file: ci/Dockerfile.rust-ci
          platforms: linux/amd64,linux/arm64
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max
```

Multi-arch build (`amd64` + `arm64`) so the image works on both GHA `ubuntu-latest` (x86) and Apple Silicon dev machines running Colima or a Linux VM.

Auth: `GITHUB_TOKEN` with `packages: write` permission (granted in the workflow-level `permissions:` block — no repository secret required).

### Local build

```bash
just build-ci-image          # builds locally, no push
just push-ci-image           # builds + pushes (requires GHCR_TOKEN with write:packages)
```

---

## xtask + Justfile targets

Logic lives in `xtask/`; the `Justfile` is a one-line shim per target:

```
# Justfile (thin shims)
build-ci-image:  cargo xtask build-ci-image
push-ci-image:   cargo xtask push-ci-image
pull-ci-image:   cargo xtask pull-ci-image
```

```rust
// xtask/src/main.rs
"build-ci-image" => {
    cmd!("docker", "build",
        "-t", "ghcr.io/<org>/minibox-rust-ci:stable",
        "-f", "ci/Dockerfile.rust-ci", ".").run()?;
}
"push-ci-image" => {
    cmd!("docker", "push", "ghcr.io/<org>/minibox-rust-ci:stable").run()?;
}
"pull-ci-image" => {
    cmd!("minibox", "pull", "ghcr.io/<org>/minibox-rust-ci:stable").run()?;
}
```

---

## Image Location

```
~/.mbx/cache/images/ghcr.io/<org>/minibox-rust-ci/stable/
  manifest.json
  layers/
    sha256:<digest>/     # debian base
    sha256:<digest>/     # rust toolchain
    sha256:<digest>/     # cargo tools
    sha256:<digest>/     # env config
```

Layer sharing: if `debian:bookworm-slim` is already cached from another pull, its layers are not re-downloaded. This is standard OCI layer deduplication via content-addressed storage.

---

## Versioning

| Tag              | Meaning                                       |
| ---------------- | --------------------------------------------- |
| `stable`         | Latest stable Rust toolchain (always updated) |
| `YYYY-MM-DD`     | Date-stamped build (e.g. `2026-03-23`)        |
| `sha-<short>`    | Pinned to specific image build (e.g. `sha-abc1234`) |

Hooks and CI reference `stable` by default. Date and SHA tags are produced automatically by `docker/metadata-action` on every publish run. Use SHA tags for reproducibility when debugging a specific build.

---

## New files

| File                             | Purpose                     |
| -------------------------------- | --------------------------- |
| `ci/Dockerfile.rust-ci`          | CI image definition         |
| `.github/workflows/image-ci.yml` | Build + publish on schedule |

---

## Success Criteria

- `minibox pull ghcr.io/<org>/minibox-rust-ci:stable` downloads and caches image in `~/.mbx/cache/`
- Image contains `cargo`, `rustfmt`, `clippy`, `cargo-nextest`, `cargo-llvm-cov`, `samply`, `cargo-deny`, `cargo-audit`
- `just build-ci-image` builds locally without pushing
- Weekly publish workflow runs without manual intervention
- Multi-arch: image runs on both `linux/amd64` and `linux/arm64`
