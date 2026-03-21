# CI Agent Hardening Design

**Date:** 2026-03-21
**Status:** Approved
**Goal:** Harden `scripts/ci_agent.py` into a reliable, module-split package with LLM provider fallback (Anthropic → OpenAI → Gemini) and Gitea issue creation (deduplicated by commit SHA).

---

## Background

The existing `scripts/ci_agent.py` is a single-file script (~150 lines) that:
- Fetches failed job logs from Gitea
- Asks Claude (Anthropic) to diagnose the failure
- Posts a one-line commit status
- Prints the full diagnosis to the CI job log

The council analysis flagged: define trigger criteria, permissions/scopes, data access boundaries, and how outputs are stored. The script also has a single point of failure (one LLM provider) and no persistent output beyond the ephemeral job log.

---

## Architecture

Three-module Python package replacing the single-file script:

```
scripts/ci_agent/
├── __main__.py      # thin orchestrator — sequences the flow
├── providers.py     # LLM fallback chain: Anthropic → OpenAI → Gemini
└── issues.py        # Gitea issue create / search / comment
```

Invocation changes from `python3 scripts/ci_agent.py` to `python3 -m scripts.ci_agent`.

---

## Module Designs

### providers.py

**Responsibility:** Abstract LLM access behind a single fallback-aware function. Each provider is independently testable. Missing API keys skip that provider silently; present keys that fail log the error and try the next.

```python
class DiagnosisUnavailable(Exception): ...

def ask_with_fallback(prompt: str) -> tuple[str, str]:
    """Try Anthropic → OpenAI → Gemini.
    Returns (diagnosis_text, provider_name_used).
    Raises DiagnosisUnavailable if all providers fail or no keys are set."""
```

**Models (current as of 2026-03):**

| Provider | Model | Input $/1M | Output $/1M |
|----------|-------|-----------|------------|
| Anthropic | `claude-sonnet-4-6` | $3.00 | $15.00 |
| OpenAI | `gpt-4.1` | $2.00 | $8.00 |
| Google | `gemini-2.5-flash` | $0.15 | $0.60 |

**Environment variables (all optional — missing key skips provider):**
- `ANTHROPIC_API_KEY`
- `OPENAI_API_KEY`
- `GEMINI_API_KEY`

**Error handling:**
- No key set → skip provider, `print(f"Skipping {name}: no API key")`
- Key present, call fails → log error, try next provider
- All providers exhausted → raise `DiagnosisUnavailable`

---

### issues.py

**Responsibility:** Gitea issue lifecycle — create, deduplicate, comment. Failures are non-fatal: log a warning, don't propagate (diagnosis is already in the job log regardless).

**Deduplication strategy:** Each created issue embeds `<!-- sha: {full_commit_sha} -->` as a hidden HTML comment in the body. To find an existing issue: search open issues with label `ci-failure`, scan results for the SHA marker. This avoids a separate index and survives Gitea restarts.

```python
def ensure_label_exists(repo: str, headers: dict) -> None:
    """Create 'ci-failure' label (color #e11d48) if absent. Idempotent."""

def find_issue_for_commit(repo: str, sha: str, headers: dict) -> int | None:
    """Return open issue number for this commit SHA, or None."""

def create_issue(
    repo: str, sha: str, diagnosis: str,
    provider: str, failed_jobs: list[str], headers: dict,
) -> int:
    """Open new issue. Returns issue number."""

def add_comment(
    repo: str, issue_number: int, diagnosis: str,
    provider: str, headers: dict,
) -> None:
    """Append re-run diagnosis as a comment."""
```

**Issue title format:** `CI failure: {sha[:8]} — {', '.join(failed_job_names)}`

**Issue body structure:**
```markdown
## CI Failure Diagnosis

**Jobs:** lint
**Provider:** anthropic/claude-sonnet-4-6
**Commit:** abc12345678...
**Run:** [#123](http://gitea/repo/actions/runs/123)

{diagnosis text}

---
*Diagnosed by ci-agent. Full logs in the run linked above.*
<!-- sha: abc12345678... -->
```

---

### __main__.py

**Responsibility:** Thin orchestrator. No business logic — just sequence the modules and handle the fallback issue path.

**Flow:**

```
1. Read env vars → fail fast if required vars missing
2. Fetch failed jobs from Gitea → exit 0 if none
3. Fetch logs per failed job (30KB tail each)
4. Build diagnosis prompt
5. ask_with_fallback(prompt)
   ├── success  → (diagnosis, provider)
   └── DiagnosisUnavailable → diagnosis = "All LLM providers failed — raw logs attached"
                               provider = "none"
6. Print diagnosis to stdout (captured in job log)
7. set_commit_status(state, one_line_summary)
8. ensure_label_exists(repo)
9. find_issue_for_commit(repo, sha)
   ├── found     → add_comment(issue_number, diagnosis, provider)
   └── not found → create_issue(sha, diagnosis, provider, failed_jobs)
```

**Required env vars** (fail fast with clear message if absent):
- `GITEA_URL`
- `CI_AGENT_TOKEN`
- `REPO`
- `RUN_ID`
- `COMMIT_SHA`

---

## Workflow Changes

Add two new secrets to the `diagnose` job in `.gitea/workflows/ci.yml`:

```yaml
- name: run diagnosis agent
  env:
    GITEA_URL: http://100.105.75.7:3000
    CI_AGENT_TOKEN: ${{ secrets.CI_AGENT_TOKEN }}
    REPO: ${{ gitea.repository }}
    RUN_ID: ${{ gitea.run_id }}
    COMMIT_SHA: ${{ gitea.sha }}
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
    OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
    GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}
  run: python3 -m scripts.ci_agent
```

---

## Permissions & Data Access

| Resource | Access | Notes |
|----------|--------|-------|
| Gitea jobs API | Read | List jobs for a run, fetch conclusion |
| Gitea logs API | Read | Fetch up to 30KB tail per failed job |
| Gitea statuses API | Write | Post `ci/agent-diagnosis` commit status |
| Gitea issues API | Read + Write | Search open issues, create issue, post comment |
| Gitea labels API | Read + Write | Create `ci-failure` label if absent |
| Anthropic API | Write | Send diagnosis prompt, receive text |
| OpenAI API | Write | Fallback only |
| Google Gemini API | Write | Fallback only |

**Data boundaries:** Only failed job log tails (30KB per job) are sent to LLM providers. No source code, secrets, or environment variables from the failing jobs are included in prompts.

---

## Error Handling Summary

| Failure | Behaviour |
|---------|-----------|
| Missing required env var | `sys.exit(1)` with clear message |
| Gitea jobs API fails | `sys.exit(1)` — can't proceed without job data |
| Gitea logs API fails | Log warning, use `"(log unavailable)"` placeholder |
| LLM provider: no key | Skip silently, try next |
| LLM provider: call fails | Log error, try next |
| All LLM providers fail | Create fallback issue with raw logs |
| Issue API fails | Log warning, don't crash |

---

## File Map

| Action | File |
|--------|------|
| Delete | `scripts/ci_agent.py` |
| Create | `scripts/ci_agent/__main__.py` |
| Create | `scripts/ci_agent/providers.py` |
| Create | `scripts/ci_agent/issues.py` |
| Modify | `.gitea/workflows/ci.yml` |
