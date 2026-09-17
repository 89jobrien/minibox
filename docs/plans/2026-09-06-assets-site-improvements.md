---
source_sha: 98cfc0e94b126624f7c86c89e8edc86444ab63c4
sources:
  - .gitignore
  - docs/core/ARCHITECTURE.mbx.md
  - docs/core/FEATURE_MATRIX.mbx.md
  - docs/core/ROADMAP.mbx.md
generated: 2026-09-16
---

# Assets Site Improvement Tracker

> **Historical snapshot:** the ignored local `assets/` tree described below is not present in
> this checkout and was never a durable repository source. The tracker is retained to explain
> that abandoned direction; current tracked site documentation lives under `docs/`.

Tracks the abandoned follow-up work for the former static reference site under `assets/`. At the
time this plan was written, those pages existed only as ignored local files because `.gitignore`
excluded the entire directory.

## Status Legend

- [ ] Not started
- [~] In progress
- [x] Complete

## P0: Make the Site Durable

### ASSET-01: Track and publish the site

- [ ] Decide whether `assets/` is source, generated output, or both.
- [ ] Remove or narrow the blanket `assets/` ignore rule for files that must ship.
- [ ] Define the publication target and reproducible publish command.
- [ ] Confirm a clean clone can build or serve the same pages.

**Done when:** site changes appear in Git review and the published site comes from tracked inputs.

### ASSET-02: Generate factual content from canonical sources

- [ ] Define which values come from Cargo metadata, protocol types, workflows, test output, and
      `docs/core/` references.
- [ ] Generate volatile facts such as versions, crate counts, protocol counts, workflow counts,
      and dated test evidence.
- [ ] Fail generation when a referenced source or expected field disappears.
- [ ] Keep narrative text hand-maintained where automation would obscure intent.

**Done when:** changing a canonical fact updates every affected page through one generation path.

## P1: Improve Maintainability and Usability

### ASSET-03: Extract shared page chrome

- [ ] Move navigation and footer markup into shared templates or includes.
- [ ] Set the active navigation item from page metadata.
- [ ] Verify every page renders identical navigation links and ordering.

**Done when:** navigation or footer changes require one edit instead of ten.

### ASSET-04: Add responsive table behavior

- [ ] Wrap wide feature, crate, testing, and stability tables in horizontal scroll containers.
- [ ] Preserve visible row labels while scrolling on narrow screens.
- [ ] Verify phone, tablet, and desktop layouts without clipped content.

**Done when:** every matrix remains readable at 320px width.

### ASSET-05: Add accessibility gates

- [ ] Validate heading order, landmarks, link purpose, table headers, and focus visibility.
- [ ] Check text, status pills, graph labels, and diagram lines for sufficient contrast.
- [ ] Add long descriptions or adjacent summaries for information-dense SVGs.
- [ ] Confirm all navigation and controls work with a keyboard.

**Done when:** automated checks pass and a keyboard-only review can reach and understand all pages.

### ASSET-06: Add visual regression coverage

- [ ] Capture deterministic desktop and mobile screenshots for every page.
- [ ] Cover the architecture diagrams and widest data tables explicitly.
- [ ] Run screenshot comparison when site sources or shared styles change.

**Depends on:** ASSET-01.

**Done when:** layout regressions are visible in review before publication.

## P2: Improve Navigation and Traceability

### ASSET-07: Add architecture-page navigation

- [ ] Add stable section IDs to the architecture walkthrough.
- [ ] Add an on-page table of contents with deep links to all architecture sections.
- [ ] Preserve the current section ordering and browser back-button behavior.

**Done when:** each architecture section has a durable shareable URL fragment.

### ASSET-08: Improve diagram readability

- [ ] Add consistent legends for production, experimental, blocked, optional, and no-op paths.
- [ ] Provide a full-size or zoomable view for dense diagrams.
- [ ] Check graph label size and line contrast on narrow and high-density displays.
- [ ] Split any graph that still requires excessive zoom to understand.

**Done when:** all graphs are legible on desktop and usable on mobile without losing context.

### ASSET-09: Add source provenance

- [ ] Record source commit, generation date, and exact source paths for each page.
- [ ] Distinguish dated measurements from durable architectural contracts.
- [ ] Surface stale-source warnings during generation or validation.

**Depends on:** ASSET-02.

**Done when:** readers and maintainers can identify where every volatile claim came from.

### ASSET-10: Format generated HTML and SVG

- [ ] Emit stable indentation and line wrapping for HTML.
- [ ] Emit readable multi-line SVG rather than single-line documents.
- [ ] Keep formatting deterministic to produce reviewable diffs.

**Depends on:** ASSET-02, ASSET-03.

**Done when:** generated changes produce small, comprehensible diffs.

### ASSET-11: Add site-wide search

- [ ] Index page headings, summaries, commands, crates, adapters, and architecture terms.
- [ ] Keep the search index local and reproducible with no hosted runtime dependency.
- [ ] Support keyboard access and direct navigation to matched sections.

**Depends on:** ASSET-01, ASSET-03.

**Done when:** a user can locate a feature, command, crate, or adapter from any page.

### ASSET-12: Link roadmap gaps to tracked work

- [ ] Associate roadmap and feature gaps with GitHub issues or explicit TODO references.
- [ ] Avoid links to completed or superseded work.
- [ ] Add a validation check for missing or malformed tracking references.

**Done when:** every actionable roadmap gap has an owner-visible tracking destination.

## Suggested Order

1. ASSET-01: establish tracked inputs and publication ownership.
2. ASSET-02 and ASSET-03: create one maintainable generation/template path.
3. ASSET-04 and ASSET-05: establish responsive and accessible output.
4. ASSET-06: protect the resulting layouts with visual regression coverage.
5. ASSET-07 through ASSET-12: add navigation, readability, provenance, search, and traceability.
