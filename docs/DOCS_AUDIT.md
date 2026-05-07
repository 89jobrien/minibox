# Documentation Audit

> Generated 2026-04-27 from automated analysis of all docs/\*.md files.
> Updated 2026-04-28: items 1-7 addressed (see Recommendations).
> Updated 2026-05-05: ARCHITECTURE, CRATE_INVENTORY, CRATE_TIERS, FEATURE_MATRIX corrected
> for 10-crate workspace, version 0.24.0, smolvm as default adapter, protocol variant counts,
> minibox-crux-plugin and minibox-conformance added throughout, dockerbox/tailbox removed from
> Platform tier table.
> Updated 2026-05-06: ARCHITECTURE (GKE ImagePusher Y), FEATURE_MATRIX (GKE push Yes,
> OTEL env vars documented), CRATE_TIERS (10 crates), STATE_MODEL (RwLock verified).

## Top-Level Docs Status

| File                           | Severity     | Key Issues                                                                                                                                 |
| ------------------------------ | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| FEATURE_MATRIX.md              | **current**  | Clean                                                                                                                                      |
| STATE_MODEL.md                 | **current**  | Clean (RwLock verified 2026-05-06)                                                                                                         |
| STABILITY_CHECKLIST.md         | **current**  | Clean                                                                                                                                      |
| CRATE_TIERS.md                 | **current**  | Clean                                                                                                                                      |
| ROADMAP.md                     | **stale**    | Only covers dogfooding; missing engineering roadmap (krun wiring, handler coverage, Windows phase 2)                                       |
| PRERELEASE_CHANGELOG.md        | **current**  | Minor: minibox-agent revert not explicitly noted; no target version on [Unreleased]                                                        |
| minibox-agent-design.md        | **outdated** | Describes removed crates (minibox-agent, minibox-llm) with no "removed" header                                                             |
| parallel-layer-pulls-design.md | **outdated** | Feature shipped in v0.1.0; all crate paths stale (minibox-lib, linuxbox)                                                                   |
| dagu-integration.md            | **outdated** | miniboxctl and minibox-dagu/ don't exist; entire workflow is non-executable                                                                |
| atif-viability-report.md       | **stale**    | All implementation targets reference removed crates                                                                                        |
| notes/ClaudeUsage.md           | **current**  | Personal note; dead cross-ref to rootDotClaudeDotJsonSchema.json                                                                           |

---

## Cross-Cutting Issues

### 1. Design archive docs need rescue headers

Three docs describe removed infrastructure or already-shipped features with no
status marker: `minibox-agent-design.md`, `parallel-layer-pulls-design.md`,
`dagu-integration.md`. Readers encountering them cold will be confused.

**Fix:** Add a prominent blockquote at the top of each:

```markdown
> NOTE: This document describes [removed/already-shipped] functionality.
> See [current location] for the implementation.
```

### 2. Test count inconsistency

CRATE_TIERS.md says "~300+ tests". STABILITY_CHECKLIST.md and CLAUDE.md say
~760. Both dated 2026-04-27. CRATE_TIERS.md was not updated during
consolidation.

### 3. minibox-plugins workspace is a dead end

PRERELEASE_CHANGELOG references `minibox-plugins` as the destination for 6
extracted crates (dashbox, dockerbox, minibox-secrets, tailbox, minibox-bench,
searchbox). This workspace does not exist at the expected path. Any
cross-reference is currently broken.

### 4. ROADMAP.md is undersized

Only covers dogfooding ideas. Missing engineering priorities: handler coverage
gate, krun daemon wiring, dockerbox shim, ATIF implementation, Windows phase 2.

### 5. dagu-integration.md is the most misleading doc

Describes a concrete runnable workflow with specific binary paths and port
numbers, but miniboxctl and minibox-dagu/ do not exist.

---

## Plans & Specs Audit

### Status Summary (50 plans)

| Status                                      | Count |
| ------------------------------------------- | ----- |
| Completed (verified)                        | 31    |
| Completed but questionable (code not found) | 5     |
| Archived/Superseded                         | 3     |
| Abandoned (no commits)                      | 2     |
| Open/Unstarted                              | 3     |

### Questionable "Done" Plans

These have `status: done` in frontmatter but the described code was not found:

| Plan                     | Issue                                                                   |
| ------------------------ | ----------------------------------------------------------------------- |
| otel-tracing-prometheus  | No `telemetry/` dir in any crate                                        |
| tailnet-integration      | No `tailbox/` crate exists                                              |
| crux-minibox-integration | No `minibox-agent/` crate; minibox-agent-reland spec contradicts "done" |
| searchbox                | No `searchbox/` or `zoektbox/` crate                                    |
| sandboxed-ai-execution   | No `mbx sandbox` subcommand or toolchain Dockerfile                     |
| slashcrux-integration    | No `slashcrux` dep in workspace Cargo.toml                              |

### Abandoned Plans (safe to delete or archive)

| Plan                                | Reason                                      |
| ----------------------------------- | ------------------------------------------- |
| 2026-03-21-vps-automation-safety.md | Archived, "no commit evidence found"        |
| 2026-03-18-parallel-layer-pulls.md  | Aborted 2026-04-23, stale crate paths       |
| gke-docker-image.md                 | No frontmatter, no status, no matching code |

### Orphaned Specs (19 specs without matching plans)

Notable ones requiring attention:

| Spec                                      | Status                                               |
| ----------------------------------------- | ---------------------------------------------------- |
| maestro-tiered-ci-design.md               | **Wrong repo** -- Maestro artifact in minibox docs   |
| minibox-agent-reland-design.md            | Draft, contradicts crux-minibox plan's "done" status |
| winbox-wsl2-proxy-design.md               | Draft, supersedes old wsl2-winboxd plan              |
| commit-build-push-conformance-boundary.md | Active spec, needs a plan                            |
| minibox-agent-runtime-design.md           | Approved spec, needs a plan                          |
| cas-runtime.md                            | Draft, needs triage                                  |

### Stale Crate References

16 plans and 5 specs reference `daemonbox`, `minibox-cli`, `minibox-client`,
`miniboxctl`, or `linuxbox` in file maps and code paths. These crate names are
all stale (absorbed/renamed). The plans are historically accurate but misleading
to future readers.

### Duplicate Spec

`agentbox-diagrams.md` (undated) duplicates `2026-03-26-agentbox-diagrams.md`.
Delete the undated copy.

---

## Recommendations (Priority Order)

1. ~~**Add rescue headers** to minibox-agent-design.md,
   parallel-layer-pulls-design.md, dagu-integration.md~~ DONE 2026-04-28
2. ~~**Fix test count** in CRATE_TIERS.md: ~300+ -> ~760+~~ DONE 2026-04-28
3. ~~**Audit 6 questionable "done" plans** against actual codebase~~
   DONE 2026-04-28: 3 confirmed done (otel, sandboxed-ai, slashcrux);
   3 changed to archived (tailnet, crux-minibox, searchbox)
4. ~~**Move maestro specs** out of minibox docs~~ DONE 2026-04-28 (moved
   to `specs/archived/`)
5. ~~**Delete undated agentbox-diagrams.md** duplicate~~ DONE 2026-04-28
6. ~~**Expand ROADMAP.md** with engineering priorities~~ DONE 2026-04-28
   (P0-P3 sections added)
7. ~~**Archive 3 abandoned plans** or delete them~~ DONE 2026-04-28:
   2 already had `status: archived`; added frontmatter to
   `gke-docker-image.md` and marked archived
8. ~~**Create plan stubs** for 5 orphaned active specs~~ DONE 2026-04-28:
   created stubs for minibox-agent-reland, winbox-wsl2-proxy,
   commit-build-push-conformance, minibox-agent-runtime, cas-runtime
9. ~~**Resolve minibox-plugins dead end**~~ DONE 2026-04-28 (CRATE_TIERS.md
   updated: dockerbox/tailbox noted as extracted to minibox-plugins)

Additionally fixed:
- Added rescue header to `atif-viability-report.md` (stale crate refs)
