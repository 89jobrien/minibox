---
source_sha: 78e6b888e7c43b7d93ac244c4123295ea59d9f89
sources:
  - crates/minibox-core
  - crates/minibox-domain
  - crates/minibox
  - crates/miniboxd
  - crates/mbx
  - crates/macbox
  - crates/smolbox
  - crates/minibox/src/adapters/colima.rs
  - crates/winbox
  - crates/minibox/src/adapters/docker_desktop.rs
  - crates/minibox-cni
  - crates/minibox-tui
generated: 2026-09-04
---

# Support Tiers

Formal support-tier definitions for minibox crates and adapters.

Last updated: 2026-09-16

See also: `docs/core/STABILITY_CHECKLIST.mbx.md` (mandatory gate list), `docs/core/FEATURE_MATRIX.mbx.md`
(per-adapter capability breakdown), `docs/core/CRATE_TIERS.mbx.md` (architectural layer
classification — Core/Platform/Experimental/Internal/External).

> **Relationship to CRATE_TIERS.mbx.md:** This document classifies by *support commitment level*
> (SLA, CI coverage, breaking-change policy). CRATE_TIERS classifies by *architectural role*
> (stable API contract, adapter suite, dev tooling). The two axes are independent: a crate can
> be Platform-tier architecturally while still being Stub-tier by support level (e.g. `winbox`),
> or Experimental-tier by support level while being Platform-tier architecturally (e.g. `macbox`
> adapters). When in doubt, CRATE_TIERS answers "where does this code live?"; this document
> answers "what guarantees does it carry?"

---

## Tier Definitions

### Tier 1 — Production

Fully supported. All six mandatory stability gates must pass continuously. Breaking changes require a
deprecation cycle of at least one minor release with a compiler or runtime warning before removal.
Security issues receive a response within 72 hours. Crates in this tier are covered by CI on every
PR and every push to `next`.

### Tier 2 — Experimental

Best-effort support. CI coverage is best-effort; gates may be partially met. Breaking changes may
occur between any release without a prior deprecation cycle, but will be noted in CHANGELOG.md.
Security issues are addressed on a best-effort basis. Promotion to Tier 1 requires meeting all
six mandatory gates plus the advisory review prompts and human sign-off (see Promotion below).

### Tier 3 — Stub

Not yet implemented or explicitly unwired. No CI coverage required. May be removed without notice
in any release. No security response commitment. Exists to document intent or reserve a namespace.

---

## Crate and Adapter Assignments

### Tier 1 — Production

| Crate / Component | Type    | Notes                                              |
| ----------------- | ------- | -------------------------------------------------- |
| `minibox-domain`  | Crate   | Canonical domain values, policies, events, and ports |
| `minibox-core`    | Crate   | Protocol/client/OCI infrastructure and compatibility re-exports |
| `minibox`         | Crate   | Linux and shared runtime adapter implementations    |
| `miniboxd`        | Binary  | Daemon process; socket server and handler dispatch |
| `minibox-cli`     | Package | User-facing `mbx` binary                           |
| `native` adapter  | Adapter | Linux namespace/cgroup/overlay runtime             |
| `gke` adapter     | Adapter | Unprivileged GKE pod runtime (proot-based)         |

### Tier 2 — Experimental

| Crate / Component | Type    | Notes                                                   |
| ----------------- | ------- | ------------------------------------------------------- |
| `macbox`          | Crate   | Colima composition plus feature-gated VZ path           |
| `smolbox`         | Crate   | Owns smolvm and krun implementations; depends on macbox |
| `smolvm` adapter  | Adapter | Default macOS adapter; implementation owned by smolbox  |
| `krun` adapter    | Adapter | Fallback VM adapter; implementation owned by smolbox    |
| `colima` adapter  | Adapter | Delegates to `nerdctl`/`limactl`; exec/logs are limited |
| `vz` adapter      | Adapter | Feature-gated and selectable, but VM boot is blocked    |
| `minibox-tui`     | Crate   | Read-only dashboard used by optional `minibox-cli/tui`  |
| `minibox-cni`     | Crate   | Opt-in native bridge networking behind the `cni` feature|

### Tier 3 — Stub

| Crate / Component        | Type    | Notes                                                       |
| ------------------------ | ------- | ----------------------------------------------------------- |
| `winbox`                 | Crate   | Windows adapter crate; Phase 2 (Named Pipe/HCS) not started |
| `winbox` adapter         | Adapter | Returns error unconditionally                               |
| `docker_desktop` adapter | Adapter | Code exists but not wired into `AdapterSuite` or the daemon |

---

## Support Criteria Summary

| Criterion                  | Tier 1 — Production                       | Tier 2 — Experimental              | Tier 3 — Stub           |
| -------------------------- | ----------------------------------------- | ---------------------------------- | ----------------------- |
| **Mandatory CI gates**     | All 6 gates must pass on every PR         | Best-effort; partial gate coverage | None required           |
| **Breaking change policy** | Deprecation cycle (min 1 minor release)   | May break without prior notice     | May be removed any time |
| **Security response**      | Within 72 hours                           | Best-effort                        | No commitment           |
| **Removal policy**         | Requires deprecation + major version bump | Noted in CHANGELOG                 | No notice required      |

The six mandatory gates are defined in `docs/core/STABILITY_CHECKLIST.mbx.md`. In-memory mock
doubles and the other qualitative checks are advisory review prompts, not a seventh hard gate.

---

## Promotion: Tier 2 → Tier 1

A Tier 2 adapter or crate may be promoted to Tier 1 when all of the following are satisfied:

1. All six mandatory stability gates pass on the promotion branch, with CI evidence and maintainer sign-off.
2. The adapter has at least one integration test that runs in CI (Gate 3).
3. Handler coverage for any new handler code meets the >= 80% function coverage threshold (Gate 2).
4. A PR is opened on the current `develop` → `staging` → `release` → `main` promotion path
   with a title prefixed `promote(<adapter>): Tier 2 → Tier 1`
   and a checklist confirming each gate.
5. A maintainer reviews and approves. Approval constitutes the human sign-off.

There is no automated promotion. A passing CI run alone is not sufficient — the maintainer review
ensures qualitative criteria (no `.unwrap()` in production paths, structured tracing, SAFETY
comments on unsafe blocks) are also met.
