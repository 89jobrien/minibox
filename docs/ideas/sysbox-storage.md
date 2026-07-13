# Sysbox Storage

Source: coder/sysbox repo, docs/quickstart/storage.md

## Shared storage across user-namespaced containers

The core problem: each container has its own user-namespace UID mapping,
so container A's root (host UID 165536) and container B's root (host UID
268994208) see different ownership on shared files.

## shiftfs

Sysbox solves this with **shiftfs** — a filesystem layer mounted over
shared storage that translates UIDs on the fly. Both containers see
`root:root` despite having different host UID mappings.

- Transparent to applications inside the container.
- Works with Docker volumes and host bind-mounts.
- Mounted automatically by sysbox-runc at container creation.

## ID-mapped mounts (kernel 5.12+)

Alternative to shiftfs on newer kernels. Same result — UID translation
at the VFS layer — but uses in-kernel mount attributes instead of a
separate filesystem module.

## Storage backends

- Docker volumes (recommended).
- Host directories via `--mount type=bind`.
- Both support simultaneous access from multiple containers.

## Relevance to minibox

- **UID translation for shared storage** is the key takeaway. If minibox
  supports user-namespace isolation and shared volumes, it needs either
  shiftfs or ID-mapped mounts to make ownership consistent across
  containers.
- **ID-mapped mounts** are the modern path (kernel 5.12+, no extra
  module). Minibox should target this since it already requires recent
  kernels for cgroup v2.
- **shiftfs** is a fallback for older kernels but adds a kernel module
  dependency — likely not worth supporting.
- The pattern of transparent UID translation at the mount layer (not
  chown at copy time) is the right design for performance and
  correctness.
