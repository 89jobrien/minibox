# Sysbox System Container Images

Source: coder/sysbox repo, docs/quickstart/images.md

## Image preloading (baking inner images into a container image)

1. Start inner dockerd during build, pull target images, stop daemon,
   commit the layer.
2. Must use legacy builder (`DOCKER_BUILDKIT=0`) — BuildKit does not
   support sysbox during build.
3. The `VOLUME /var/lib/docker` directive in `docker:dind` images
   silently masks the writable layer data. Remove it if using as a base.

## Stale PID file gotcha

After commit or restart, these files persist and block daemon startup:

- `/var/run/docker.pid`
- `/run/docker/containerd/containerd.pid`

Systemd-managed images handle cleanup automatically. Manual-start images
need explicit `rm -f` before restarting dockerd.

## Commit constraints

- Cannot commit a system container while inner containers are running.
- `docker commit` captures only the writable layer — volume and
  bind-mount contents are not included.
- Use `--pause=true` (default) during commit.

## Runtime configuration for builds

Set `sysbox-runc` as the default runtime in `/etc/docker/daemon.json`
so intermediate build containers also run as system containers:

```json
{
  "default-runtime": "sysbox-runc",
  "runtimes": {
    "sysbox-runc": { "path": "/usr/bin/sysbox-runc" }
  }
}
```

## Relevance to minibox

- The `VOLUME` directive masking layer data is a trap if minibox ever
  supports image preloading or commit workflows — the writable layer
  silently loses data when a volume is declared over it.
- PID file cleanup pattern applies to any init/daemon lifecycle
  management inside containers.
- Commit-only-captures-writable-layer constraint is relevant to
  minibox's overlay storage model.
