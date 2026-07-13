# Docker-in-Docker via Sysbox

Source: coder/sysbox repo, docs/quickstart/dind.md

## How it works

`--runtime=sysbox-runc` replaces the OCI runtime, giving each container
its own isolated user namespace + virtual procfs/sysfs. Inner Docker runs
unprivileged — no `--privileged` flag needed.

## Run pattern

```bash
docker run --runtime=sysbox-runc -it --rm \
  --mount source=dind-vol,target=/var/lib/docker \
  nestybox/alpine-docker
```

## Critical constraints

- Each nested Docker instance needs its own dedicated `/var/lib/docker` —
  cannot be shared across containers.
- Bind-mounting `/var/lib/docker` masks any preloaded images baked into
  the container image.
- Docker volumes preserve preloaded images; bind mounts do not.
- Alpine-based images require manual `dockerd` startup; systemd-based
  images auto-start.

## Inner Docker startup (non-systemd)

```bash
dockerd > /var/log/dockerd.log 2>&1 &
docker pull alpine  # works inside the container
```

## Multi-container image caching

Use a local registry as a pull-through cache rather than trying to share
`/var/lib/docker`.

## Available base images

| Image                                       | Init system  | Auto-starts Docker |
| ------------------------------------------- | ------------ | ------------------ |
| `nestybox/alpine-docker`                    | none         | no                 |
| `nestybox/ubuntu-bionic-systemd-docker`     | systemd      | yes                |
| `nestybox/alpine-supervisord-docker`        | supervisord  | yes                |

## Relevance to minibox

- Dedicated-datastore-per-instance constraint maps directly to minibox's
  per-container overlay storage model.
- The sysbox-runc runtime swap pattern is worth studying if minibox ever
  supports nested container runtimes or DinD scenarios.
- Volume vs bind-mount tradeoff for image persistence applies to
  overlay/image storage design decisions.
