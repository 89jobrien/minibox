---
status: archived
note: "No frontmatter, no status, no matching code found"
---

# Plan: miniboxd Docker Image for GKE

## Context

miniboxd has a fully implemented GKE adapter (`ProotRuntime` + `CopyFilesystem` +
`NoopLimiter` + `NoopNetwork`) but no Dockerfile or container image exists. Goal:
build a Docker image, push to GHCR, deploy to GKE, `kubectl exec` in and test.

## Files to Create

1. **`Dockerfile`** (project root) — multi-stage build
2. **`.dockerignore`** — exclude target/, .git/, .worktrees/

## Files to Read (no modify)

- `crates/miniboxd/src/main.rs` — adapter dispatch (GKE path at ~L490)
- `crates/minibox/src/adapters/gke.rs` — proot dependency

## Dockerfile Design

### Stage 1: Builder
- Base: `rust:1.85-alpine` (musl target built-in)
- Install: `musl-dev`, `pkgconfig`, `openssl-dev` (for reqwest)
- `cargo build --release -p miniboxd -p mbx`
- No cargo-chef (not worth the complexity for now)

### Stage 2: Runtime
- Base: `alpine:3.21`
- Install: `proot` (available in Alpine community repo)
- Copy `miniboxd` and `mbx` from builder
- Create non-root user `minibox` (GKE adapter doesn't need root)
- Env defaults: `MINIBOX_ADAPTER=gke`, `RUST_LOG=info`
- Volumes: `/run/minibox`, `/var/lib/minibox`
- Entrypoint: `/usr/local/bin/miniboxd`

## Build & Push

```bash
# Build
docker build -t ghcr.io/89jobrien/minibox:dev .

# Push (requires gh auth)
docker push ghcr.io/89jobrien/minibox:dev
```

## GKE Deployment (quick test)

```bash
kubectl run minibox --image=ghcr.io/89jobrien/minibox:dev \
  --restart=Never --command -- sleep infinity

# Then exec in
kubectl exec -it minibox -- /bin/sh

# Inside the pod, miniboxd is available:
miniboxd &
mbx pull alpine
mbx run alpine -- /bin/echo "hello from GKE"
```

For a proper deployment later, a Deployment + PVC manifest can be added, but
for "see what happens" testing, a bare pod with `sleep infinity` + manual
daemon start is fastest.

## Verification

1. `docker build` succeeds locally
2. Image runs locally: `docker run --rm -it ghcr.io/89jobrien/minibox:dev sh`
   - Verify `miniboxd --help` and `mbx --help` work
   - Verify `proot --version` is available
3. Push to GHCR succeeds
4. `kubectl run` + `kubectl exec` into the pod
5. `miniboxd &` starts without error (MINIBOX_ADAPTER=gke)
6. `mbx pull alpine` succeeds
7. `mbx run alpine -- /bin/echo hello` prints "hello"
