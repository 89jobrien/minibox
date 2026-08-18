---
name: docs
description: Write, generate, or update project documentation in this repo, and audit existing docs for staleness, dead links, and misplaced files. Enforces where a doc lives (.ctx/memory-bank/ for agent memory, top-level ALL-CAPS.md for repo docs, docs/core/*.mbx.md for reference docs), stamps every generated doc with YAML frontmatter recording the source commit SHA it was derived from, verifies links with real HTTP requests, and finishes by naming every doc whose source files have changed since that SHA. Make sure to use this whenever the user asks to write/update/refresh/generate a doc, README, CHANGELOG, architecture or testing doc, memory-bank file, or asks "are the docs stale", "check the docs", "which docs need updating", "do the links still work", or where a doc should live — including when they name a specific file like TESTING.md or activeContext.mbx.md rather than saying "docs".
---

# Docs

Documentation in this repo is treated as a build artifact with a provenance
record, not as prose that drifts quietly. Three rules carry almost all the
value: a doc has exactly one correct home, a generated doc records the commit
it was generated from, and every claim about an external URL is checked rather
than assumed.

## Where a doc lives

Three homes, each with a different audience. Putting a doc in the wrong one is
the most common mistake because the content often reads fine either way.

| Home                     | Contents                                                                                                                                 | Audience                                                                              |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `.ctx/memory-bank/`      | `activeContext.mbx.md`, `progress.mbx.md`, `projectbrief.mbx.md`, `systemPatterns.mbx.md`, `techContext.mbx.md`, `productContext.mbx.md` | Agents resuming work. Session state, current focus, what's in flight.                 |
| Top level, `ALL-CAPS.md` | `README.md`, `CHANGELOG.md`, `TESTING.md`, `CONTRIBUTING.md`, `USAGE.md`, `DEVELOPMENT.md`, `AGENTS.md`, `CLAUDE.md`                     | Humans arriving at the repo. Entry points someone expects to find without being told. |
| `docs/core/*.mbx.md`     | `ARCHITECTURE`, `GOTCHAS`, `SECURITY_INVARIANTS`, `CRATE_INVENTORY`, `STATE_MODEL`, …                                                    | Humans and agents going deep on one subsystem. Reference material, not entry points.  |

Deciding between top level and `docs/core/`: ask whether a newcomer would look
for it before they know the codebase. `TESTING.md` answers "how do I run the
tests" — top level. `TEST_INFRASTRUCTURE.mbx.md` explains how the harness is
built — `docs/core/`.

**One topic, one home.** A base name appearing in both top level and
`docs/core/` is a defect, not a style choice: the two copies drift, and readers
have no way to tell which is authoritative. The audit script reports these as
errors. This repo currently has three (`USAGE`, `TESTING`, `DEVELOPMENT`) —
when you touch either side of a collision, resolve it rather than deepening it:
pick the canonical file, reduce the other to a one-line pointer, or merge them.

New memory-bank and `docs/core` files use the `.mbx.md` extension; top-level
files use plain `.md`.

## Freshness frontmatter

Any doc you generate or substantially regenerate from source carries YAML
frontmatter naming the commit it was derived from:

```yaml
---
source_sha: 4add60894f29527483dd1200aae362faad78f589
sources:
  - crates/minibox-core/src/protocol.rs
  - crates/minibox/src/daemon/handler/
generated: 2026-08-18
---
```

- `source_sha` — full SHA of `HEAD` at generation time. Get it with
  `git rev-parse HEAD`. A short SHA works but the full one is unambiguous
  forever; abbreviations can collide as the repo grows.
- `sources` — the files or directories you actually read to write the doc.
  This is the load-bearing field: it's what makes staleness detectable. List
  the real inputs, not the whole crate. A doc claiming to derive from
  `crates/` will be reported stale on every unrelated commit and you'll learn
  to ignore it.
- `generated` — the date, for humans skimming.

Frontmatter coexists with the existing `Last updated:` line that
`cargo xtask pre-commit`'s doc-dates step rewrites automatically under `docs/`.
Leave that line alone; it's stamped by tooling, and the frontmatter answers a
different question — not "when did someone touch this" but "what was the code
doing when this was true".

Hand-written docs (`README.md`, `CONTRIBUTING.md`) don't need frontmatter —
nothing generated them, so there's no source to drift from. The audit lists
them as unstamped for visibility, which is informational, not a failure.

## Links are checked, not assumed

Before claiming a link works, request it. A 404 in documentation is worse than
no link: it costs the reader time and signals the doc is unmaintained.

`scripts/doc-audit.nu --links` does this across every doc, deduplicating URLs
so a link repeated in ten files costs one request. Anything that doesn't
answer 2xx/3xx is reported with its status code and the docs containing it.

For a link you're adding right now, check it directly rather than running the
full sweep:

```bash
curl -sS -L -o /dev/null -w "%{http_code}" --max-time 15 <url>
```

Treat a timeout or a 000 as unverified, not as broken — some hosts refuse
automated requests. Say so rather than silently deleting the link.

## Workflow

When writing or updating a doc:

1. **Place it.** Confirm the home against the table above. If the topic
   already exists in another home, resolve that before adding to the pile.
2. **Read the actual sources.** Docs asserting things about code need to be
   grounded in that code as it exists now, not in what the previous version of
   the doc said. Note the paths you read — they become `sources`.
3. **Write it**, then stamp the frontmatter with `git rev-parse HEAD` and
   those paths.
4. **Check any links** you added.
5. **Run the audit** and report what it finds.

## Always finish with the audit

End every documentation task by running:

```bash
nu .claude/skills/docs/scripts/doc-audit.nu
nu .claude/skills/docs/scripts/doc-audit.nu --links   # when links changed or on request
```

Then tell the user what it found — specifically, **which docs have sources
that changed after the doc's `source_sha`**. This is the part that makes the
convention worth having: without it, stamping SHAs is bookkeeping nobody reads.

The script reports four things and exits non-zero if any of the first three
appear:

- **STALE** — a source file picked up a commit descended from the doc's
  `source_sha`. Reported with the commit and its subject so you can judge
  whether the change actually affects the doc. It often doesn't; a rename or a
  formatting pass won't invalidate prose. Say which ones look like real drift
  rather than dumping the list.
- **COLLISIONS** — same base name in two homes.
- **DEAD LINKS** — non-2xx/3xx, with status codes (only with `--links`).
- **UNSTAMPED** — no frontmatter, so freshness can't be checked. Informational.

A doc whose `source_sha` isn't a commit in the repo is reported explicitly
rather than passing — a typo'd or rebased-away SHA would otherwise make a doc
look permanently fresh.

`--json` emits the same data for another tool to consume.

Don't auto-fix everything the audit finds. Staleness is a judgment call about
whether the underlying change matters, and the user may be mid-way through
related work. Report, recommend, and ask — unless they've said to go ahead.
