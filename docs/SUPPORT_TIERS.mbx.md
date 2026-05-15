# Support Tiers

Formal support-tier definitions for minibox crates and adapters.

Last updated: 2026-05-14

See also: `docs/STABILITY_CHECKLIST.mbx.md` (mandatory gate list), `docs/FEATURE_MATRIX.mbx.md`
(per-adapter capability breakdown).

---

## Tier Definitions

### Tier 1 — Production

Fully supported. All 7 mandatory stability gates must pass continuously. Breaking changes require a
deprecation cycle of at least one minor release with a compiler or runtime warning before removal.
Security issues receive a response within 72 hours. Crates in this tier are covered by CI on every
PR and every push to `next`.

### Tier 2 — Experimental

Best-effort support. CI coverage is best-effort; gates may be partially met. Breaking changes may
occur between any release without a prior deprecation cycle, but will be noted in CHANGELOG.md.
Security issues are addressed on a best-effort basis. Promotion to Tier 1 requires meeting all
7 mandatory gates plus a human reviewer sign-off (see Promotion section below).

### Tier 3 — Stub

Not yet implemented or explicitly unwired. No CI coverage required. May be removed without notice
in any release. No security response commitment. Exists to document intent or reserve a namespace.

---

## Crate and Adapter Assignments

### Tier 1 — Production

| Crate / Component | Type    | Notes                                              |
| ----------------- | ------- | -------------------------------------------------- |
| `minibox-core`    | Crate   | Domain types, protocol, ports — zero OS deps       |
| `minibox`         | Crate   | Adapter implementations and domain port wiring     |
| `miniboxd`        | Binary  | Daemon process; socket server and handler dispatch |
| `mbx` (CLI)       | Binary  | User-facing CLI (`minibox-cli` crate)              |
| `native` adapter  | Adapter | Linux namespace/cgroup/overlay runtime             |
| `gke` adapter     | Adapter | Unprivileged GKE pod runtime (proot-based)         |

### Tier 2 — Experimental

| Crate / Component | Type    | Notes                                                       |
| ----------------- | ------- | ----------------------------------------------------------- |
| `macbox`          | Crate   | macOS VM adapter crate (`krun` adapter lives here)          |
| `smolvm` adapter  | Adapter | Default macOS adapter; subsecond-boot Linux VM              |
| `krun` adapter    | Adapter | libkrun-based fallback; 31 conformance tests pass           |
| `colima` adapter  | Adapter | Delegates to `nerdctl`/`limactl`; exec/logs are limited     |

### Tier 3 — Stub

| Crate / Component        | Type    | Notes                                                        |
| ------------------------ | ------- | ------------------------------------------------------------ |
| `winbox`                 | Crate   | Windows adapter crate; Phase 2 (Named Pipe/HCS) not started  |
| `winbox` adapter         | Adapter | Returns error unconditionally                                |
| `docker_desktop` adapter | Adapter | Code exists but not wired into `AdapterSuite` or the daemon  |

---

## Support Criteria Summary

| Criterion                 | Tier 1 — Production                        | Tier 2 — Experimental             | Tier 3 — Stub      |
| ------------------------- | ------------------------------------------ | ---------------------------------- | ------------------- |
| **Mandatory CI gates**    | All 7 gates must pass on every PR          | Best-effort; partial gate coverage | None required       |
| **Breaking change policy**| Deprecation cycle (min 1 minor release)    | May break without prior notice     | May be removed any time |
| **Security response**     | Within 72 hours                            | Best-effort                        | No commitment       |
| **Removal policy**        | Requires deprecation + major version bump  | Noted in CHANGELOG                 | No notice required  |

The 7 mandatory gates are defined in `docs/STABILITY_CHECKLIST.mbx.md`. Gates 1–6 are hard
blockers enforced by CI; Gate 7 (in-memory mock double) is advisory but required for Tier 1
promotion via human review.

---

## Promotion: Tier 2 → Tier 1

A Tier 2 adapter or crate may be promoted to Tier 1 when all of the following are satisfied:

1. All 7 mandatory stability gates pass on the `next` branch (Gates 1–6 via CI; Gate 7 via PR
   reviewer sign-off).
2. The adapter has at least one integration test that runs in CI (Gate 3).
3. Handler coverage for any new handler code meets the >= 80% function coverage threshold (Gate 2).
4. A PR is opened targeting `next` with a title prefixed `promote(<adapter>): Tier 2 → Tier 1`
   and a checklist confirming each gate.
5. A maintainer reviews and approves. Approval constitutes the human sign-off.

There is no automated promotion. A passing CI run alone is not sufficient — the maintainer review
ensures qualitative criteria (no `.unwrap()` in production paths, structured tracing, SAFETY
comments on unsafe blocks) are also met.
