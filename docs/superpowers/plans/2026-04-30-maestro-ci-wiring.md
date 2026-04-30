# Plan: Maestro CI Integration with Minibox

**Date:** 2026-04-30
**Status:** pending
**Scope:** Both `minibox` and `maestro` workspaces

## Context

The archived `2026-03-18-maestro-tiered-ci-design.md` plan described applying minibox's CI
tier pattern to Maestro. That plan was approved but applied to Maestro's internal Rust
workspace CI structure only — it did not address using minibox as a container backend.

This plan supersedes it. It covers two distinct integration points:

1. **Maestro consuming minibox as a `ContainerProvider`** — Maestro's local dev workflow
   uses `mbx` (or the daemon client) instead of Docker/nerdctl directly for building and
   running containers during session development.

2. **Minibox CI running on a Maestro-style GKE self-hosted runner** — the minibox GHA
   self-hosted runner on the VPS gains a GKE-compatible configuration so CI jobs can be
   delegated there.

The Toptal Maestro repo is an employer project. Changes there must follow the Jira/PR
workflow (see `maestro/CLAUDE.md`). Only the minibox-side adapter and client changes are
scoped to this plan.

---

## Part A: Minibox as Maestro ContainerProvider

### Current Maestro container workflow

Maestro sessions run in GKE pods (`maestro-session` namespace). Local dev uses:

- `maestro-docker/` — Docker compose setup for local service deps
- `maestro-runtime/` — the session runtime binary, built and pushed via `cloudbuild.yaml`
- `maestro-devcontainer/` — VS Code devcontainer for the full dev environment

The build-and-push cycle today is: `docker build` → `docker push` → deploy to GKE staging.

**Minibox default adapter context:** minibox defaults to `smolvm` (falls back to `krun`
when `smolvm` binary is absent). On macOS the Colima adapter is an opt-in alternative
(`MINIBOX_ADAPTER=colima`). The `mbx build` command added in this plan must work with the
smolvm/krun defaults, not only with the native Linux adapter.

### Minibox integration point

Minibox does not replace the GKE deployment pipeline. It replaces the local
`docker build` + `docker push` step for devs who want to use minibox as their local
container backend.

**Target workflow:**

```
mbx build <context> --tag us-east1-docker.pkg.dev/toptal-maestro/maestro/<svc>:dev
mbx push   us-east1-docker.pkg.dev/toptal-maestro/maestro/<svc>:dev
```

This requires:

- `ImageBuilder` trait implemented for the macOS Colima adapter (not yet done — see Task 3
  in the adapter-wiring plan)
- `mbx build` CLI command (does not yet exist)
- Registry credentials flow: Google Artifact Registry uses short-lived OAuth tokens obtained
  via `gcloud auth print-access-token`. The `RegistryCredentials::Token(String)` variant
  already exists in `minibox-core/src/domain.rs`. `OciPushAdapter` does not yet consume it
  (it only handles `Basic` and `Anonymous`). This must be wired.

### Tasks

#### Task A1 — `RegistryCredentials::Token` support in OciPushAdapter

**File:** `crates/minibox/src/adapters/push.rs`

Wire `Token` variant: use the token as a `Bearer` HTTP auth header in all registry API
calls. The `RegistryClient` already accepts `Authorization: Bearer <token>` in its HTTP
client — confirm and add a test.

#### Task A2 — `mbx build` command

**File:** `crates/mbx/src/commands/build.rs` (new)

CLI wrapper for a `BuildImage` daemon request (new protocol variant). Accepts:
- `--tag <ref>` — output image ref
- `--file <path>` — Dockerfile (default: `./Dockerfile`)
- `<context>` — build context directory

Sends `BuildImage { context_tar: Vec<u8>, tag: String, dockerfile: Option<String> }`
request over the Unix socket. Daemon routes to the active `ImageBuilder` adapter.

Add `BuildImage` to `DaemonRequest` in `minibox-core/src/protocol.rs` with `#[serde(default)]`
on all fields. Add snapshot test.

#### Task A3 — `ImageBuilder` for native Linux adapter

**File:** `crates/minibox/src/adapters/builder.rs`

The existing `NativeImageBuilder` stub must be completed. A minimal implementation:
1. Extract the context tar to a temp dir
2. Parse `FROM <base>` line from Dockerfile
3. Pull base image if not cached
4. Run each `RUN` and `COPY` instruction inside an overlay container (namespace + pivot_root)
5. Commit the resulting container state as the output image

For the Maestro use case (building Go binaries), the Dockerfile is typically
`FROM golang:1.23-alpine` + `COPY` + `RUN go build`. Full Dockerfile DSL is out of scope —
support `FROM`, `RUN`, `COPY`, `WORKDIR`, `ENV`, `EXPOSE`, `CMD`. Reject unsupported
instructions with a clear error.

#### Task A4 — `ImageBuilder` for Colima adapter

Delegate to `nerdctl build` inside the Lima VM. Simpler than native because Colima already
has a full container runtime. Accept context dir path (not tar — use shared Lima mount).

---

## Part B: Minibox CI on GKE-compatible Runner

The minibox self-hosted runner currently runs on the VPS (`$INFRA_VPS_HOST`) via the
`minibox-ci` skill. For GKE-compatible testing (adapter = `gke`, proot-based, no root),
the runner needs a GKE-like environment.

### Tasks

#### Task B1 — GKE runner profile in `xtask`

**File:** `crates/xtask/src/main.rs` (or new `crates/xtask/src/runner.rs`)

Add `cargo xtask test-gke-profile` that:
1. Sets `MINIBOX_ADAPTER=gke`
2. Runs `cargo xtask test-unit` with the gke adapter wired
3. Reports which tests were skipped due to missing privileges

This can run on any host without root — GKE adapter uses proot + copy FS.

#### Task B2 — CI job: `test-gke-adapter`

**File:** `.github/workflows/ci.yml` (Bash heredoc)

Add a new job that runs `cargo xtask test-gke-profile` on `ubuntu-latest`. This validates
the GKE adapter path on every PR — currently it has no CI coverage.

---

## Acceptance Criteria

- [ ] `RegistryCredentials::Token` supported in `OciPushAdapter` with HTTP Bearer auth
- [ ] `mbx build` command compiles and sends `BuildImage` request to daemon
- [ ] `NativeImageBuilder` handles `FROM`/`RUN`/`COPY`/`WORKDIR`/`ENV`/`CMD` instructions
- [ ] `ColimaNativeImageBuilder` delegates to `nerdctl build` inside Lima VM
- [ ] `cargo xtask test-gke-profile` exits 0 on Ubuntu without root
- [ ] `test-gke-adapter` CI job added to `ci.yml`, required for merge on `next`/`stable`

## Out of Scope

- Replacing `cloudbuild.yaml` in the maestro repo
- GKE pod deployment pipeline changes
- Windows build support
