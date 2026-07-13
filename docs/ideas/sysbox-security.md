# Sysbox Security Model

Source: coder/sysbox repo, docs/quickstart/security.md

## Core principle

All namespaces (including user and cgroup) are mandatory — not optional.
Containers cannot run without user-namespace isolation. This is
architectural, enforced by the runtime.

## User namespace UID mapping

- Container UID 0 maps to an unprivileged host range
  (e.g. 268994208-269059744).
- Root capabilities only apply within the container's own resources.
- **CE limitation**: all containers share identical UID mappings, so a
  process escape can see another container's files.
- **EE fix**: each container gets exclusive UID/GID ranges, preventing
  cross-container access on escape.

## Capabilities inside containers

Root processes retain full capability sets (`0000003fffffffff`), but
capabilities are scoped to container-assigned resources only. Standard
Docker gives a reduced set (`00000000a80425fb`) but with host-level
scope on escape.

Tradeoff: broader caps, narrower blast radius.

## Mount immutability

Sysbox intercepts mount syscalls and enforces restrictions on initial
mounts:

| Mount type         | Restriction                                  |
| ------------------ | -------------------------------------------- |
| Initial read-only  | Cannot be remounted read-write               |
| Initial read-write | Can be remounted read-only (one-way)         |
| Bind mounts        | Inherit source restrictions                  |
| New mounts         | Unrestricted (created after init)            |
| Unmounts           | Allowed by default (for systemd); controlled |
|                    | via `allow-immutable-unmounts=false`          |

## Privileged containers vs Sysbox

| Property                | `--privileged`       | `sysbox-runc`              |
| ----------------------- | -------------------- | -------------------------- |
| User namespace          | none (shares host)   | mandatory, UID remapped    |
| Capabilities            | all, host-scoped     | all, container-scoped      |
| Seccomp/AppArmor        | disabled             | active                     |
| Escape impact           | full host root       | unprivileged host user     |
| Mount restrictions      | none                 | syscall-intercepted        |

## Gotchas

- CE shared UID mappings mean cross-container file visibility on escape.
- Default `allow-immutable-unmounts=true` is permissive — systemd needs
  it, but it weakens mount immutability guarantees.
- Syscall interception details (whitelist vs blacklist) are not
  documented.

## Relevance to minibox

- Mandatory user-namespace isolation is the same design goal minibox
  should target — never optional, always on.
- The mount-immutability interception pattern (trap mount syscalls,
  enforce policy) is directly applicable to minibox's overlay and
  bind-mount security.
- Shared vs exclusive UID ranges is a design decision minibox will face
  if supporting multi-tenant containers. Exclusive ranges are safer but
  consume more host UID space.
- "Broader caps, narrower blast radius" is a useful framing for
  minibox's capability model — give containers what they need, but
  scope it to their namespace.
