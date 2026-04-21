# minibox

Linux-specific container runtime: namespaces, cgroups v2, overlay filesystem, and process management.

## Modules

- **container/** — Namespace setup, cgroup creation, overlay mounts, pivot_root, exec
- **image/** — OCI layer extraction with path traversal protection, image reference parsing, Docker Hub registry client
- **adapters/** — Platform-specific adapter implementations (native Linux, colima, gke, docker-desktop, ghcr, vf, hcs, wsl2) plus `NativeImageLoader` for loading OCI tarballs into the image store
- **domain.rs** — Re-exports all `minibox-core` domain traits

## Key Functions

- `create_overlay()` — Set up readonly layers + read-write container filesystem
- `setup_cgroup()` — Create cgroup v2 group and apply memory/CPU limits
- `create_container_namespaces()` — Fork child in isolated PID/mount/network/UTS/IPC/user namespace
- `extract_layer()` — Safely extract tar.gz layer with validation against path traversal attacks

## Adapters

Select runtime with `MINIBOX_ADAPTER` env var:

- `native` — Linux namespaces, overlay, cgroups v2 (requires root, Linux only)
- `gke` — proot + copy filesystem (unprivileged, GKE-safe)
- `colima` — macOS via limactl/nerdctl

## Security

All paths derived from tar entries or user input are validated via `validate_layer_path()`. Symlinks are rewritten to prevent absolute traversal. Device nodes, pipes, and setuid/setgid bits are stripped before extraction.
