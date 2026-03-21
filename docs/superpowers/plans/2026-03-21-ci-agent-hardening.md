# CI Agent Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `scripts/ci_agent.py` with a reliable module-split package that adds LLM provider fallback (Anthropic → OpenAI → Gemini) and persists diagnoses as Gitea issues deduplicated by commit SHA.

**Architecture:** Three Python modules — `providers.py` (LLM fallback chain), `issues.py` (all Gitea API calls including commit status), `__main__.py` (thin orchestrator). Tests use pytest via uv. HTTP calls are patched with `unittest.mock.patch("urllib.request.urlopen")` — no real HTTP, no external services.

**Tech Stack:** Python 3 stdlib only for production (`urllib`, `json`, `os`, `sys`). `uv` for dev tooling. `pytest` (dev dep only). Uses Gitea REST API v1, Anthropic Messages API, OpenAI Chat Completions API, Google Gemini generateContent API.

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Create | `scripts/pyproject.toml` | uv project — pytest dev dep, test discovery config |
| Create | `scripts/__init__.py` | Empty — makes `scripts/` a Python package |
| Create | `scripts/ci_agent/__init__.py` | Empty — makes `scripts/ci_agent/` a package |
| Create | `scripts/ci_agent/providers.py` | LLM fallback chain, `DiagnosisUnavailable` |
| Create | `scripts/ci_agent/issues.py` | Gitea issue lifecycle + commit status |
| Create | `scripts/ci_agent/__main__.py` | Orchestrator — env vars, flow, exits |
| Create | `scripts/tests/__init__.py` | Empty — makes tests directory a package |
| Create | `scripts/tests/test_providers.py` | Unit tests for providers.py |
| Create | `scripts/tests/test_issues.py` | Unit tests for issues.py |
| Create | `scripts/tests/test_main.py` | Integration test for __main__ flow |
| Delete | `scripts/ci_agent.py` | Replaced by the package |
| Modify | `.gitea/workflows/ci.yml` | Add OPENAI/GEMINI keys, update invocation |

---

## Task 1: Scaffold Package Structure and uv Project

**Files:**
- Create: `scripts/pyproject.toml`
- Create: `scripts/__init__.py`
- Create: `scripts/ci_agent/__init__.py`
- Create: `scripts/tests/__init__.py`

- [ ] **Step 1: Create pyproject.toml for the scripts package**

```toml
# scripts/pyproject.toml
[project]
name = "minibox-scripts"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = []

[dependency-groups]
dev = ["pytest>=8"]

[tool.pytest.ini_options]
testpaths = ["tests"]
```

- [ ] **Step 2: Create the empty init files**

```bash
touch scripts/__init__.py
touch scripts/tests/__init__.py
mkdir -p scripts/ci_agent
touch scripts/ci_agent/__init__.py
```

- [ ] **Step 3: Install dev dependencies**

```bash
cd scripts && uv sync
```

Expected: uv creates `.venv/` inside `scripts/` and installs pytest.

- [ ] **Step 4: Verify pytest is available**

```bash
cd scripts && uv run pytest --version
```

Expected: `pytest 8.x.x`

- [ ] **Step 5: Commit**

```bash
git add scripts/pyproject.toml scripts/__init__.py scripts/ci_agent/__init__.py scripts/tests/__init__.py
git commit -m "chore(ci-agent): scaffold package structure with uv + pytest"
```

---

## Task 2: providers.py — Anthropic

**Files:**
- Create: `scripts/ci_agent/providers.py`
- Create: `scripts/tests/test_providers.py`

All `uv run pytest` commands must be run from the `scripts/` directory.

- [ ] **Step 1: Write the failing test**

Create `scripts/tests/test_providers.py`:

```python
import json
import os
import urllib.error
import unittest
from unittest.mock import MagicMock, patch

from ci_agent.providers import DiagnosisUnavailable, ask_with_fallback


def _mock_response(body: dict) -> MagicMock:
    m = MagicMock()
    m.read.return_value = json.dumps(body).encode()
    m.__enter__ = lambda s: s
    m.__exit__ = MagicMock(return_value=False)
    return m


ANTHROPIC_RESPONSE = {
    "content": [{"type": "text", "text": "Root cause: missing dep"}]
}


class TestAnthropic(unittest.TestCase):
    @patch.dict(os.environ, {"ANTHROPIC_API_KEY": "sk-test"}, clear=False)
    @patch("urllib.request.urlopen")
    def test_anthropic_returns_diagnosis(self, mock_urlopen):
        mock_urlopen.return_value = _mock_response(ANTHROPIC_RESPONSE)
        diagnosis, provider = ask_with_fallback("diagnose this")
        self.assertEqual(diagnosis, "Root cause: missing dep")
        self.assertEqual(provider, "anthropic/claude-sonnet-4-6")

    @patch.dict(os.environ, {}, clear=True)
    def test_no_keys_raises(self):
        with self.assertRaises(DiagnosisUnavailable):
            ask_with_fallback("diagnose this")
```

Note: imports use `from ci_agent.providers` (not `scripts.ci_agent.providers`) because pytest runs from inside `scripts/`.

- [ ] **Step 2: Run to confirm it fails**

```bash
cd scripts && uv run pytest tests/test_providers.py -v 2>&1 | head -20
```

Expected: `ImportError` or `ModuleNotFoundError` — `providers.py` doesn't exist yet.

- [ ] **Step 3: Create `scripts/ci_agent/providers.py` with Anthropic only**

```python
"""LLM provider fallback chain: Anthropic → OpenAI → Gemini."""

import json
import os
import urllib.request
import urllib.error


class DiagnosisUnavailable(Exception):
    """Raised when all configured LLM providers fail."""


MAX_TOKENS = 1024


def _ask_anthropic(prompt: str) -> str:
    key = os.environ.get("ANTHROPIC_API_KEY")
    if not key:
        raise ValueError("no key")
    body = json.dumps({
        "model": "claude-sonnet-4-6",
        "max_tokens": MAX_TOKENS,
        "messages": [{"role": "user", "content": prompt}],
    }).encode()
    req = urllib.request.Request(
        "https://api.anthropic.com/v1/messages",
        data=body,
        headers={
            "x-api-key": key,
            "anthropic-version": "2023-06-01",
            "content-type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        resp = json.loads(r.read())
    return next(b["text"] for b in resp["content"] if b["type"] == "text")


def ask_with_fallback(prompt: str) -> tuple[str, str]:
    """Try Anthropic → OpenAI → Gemini.
    Returns (diagnosis_text, provider_name_used).
    Raises DiagnosisUnavailable if all providers fail or no keys are set."""
    providers = [
        ("anthropic/claude-sonnet-4-6", _ask_anthropic),
    ]
    errors = []
    for name, fn in providers:
        try:
            text = fn(prompt)
            return text, name
        except ValueError:
            print(f"Skipping {name}: no API key")
        except Exception as e:
            print(f"Provider {name} failed: {e}")
            errors.append(f"{name}: {e}")
    raise DiagnosisUnavailable(f"All providers failed: {'; '.join(errors)}")
```

- [ ] **Step 4: Run tests — both should pass**

```bash
cd scripts && uv run pytest tests/test_providers.py::TestAnthropic -v
```

Expected: `PASSED` for both `test_anthropic_returns_diagnosis` and `test_no_keys_raises`.

---

## Task 3: providers.py — OpenAI and Gemini fallback

**Files:**
- Modify: `scripts/ci_agent/providers.py`
- Modify: `scripts/tests/test_providers.py`

- [ ] **Step 1: Add OpenAI and Gemini tests**

Append to `scripts/tests/test_providers.py`:

```python
OPENAI_RESPONSE = {
    "choices": [{"message": {"content": "Root cause: bad config"}}]
}

GEMINI_RESPONSE = {
    "candidates": [{"content": {"parts": [{"text": "Root cause: disk full"}]}}]
}


class TestFallbackChain(unittest.TestCase):
    @patch.dict(os.environ, {"ANTHROPIC_API_KEY": "sk-a", "OPENAI_API_KEY": "sk-o"}, clear=False)
    @patch("urllib.request.urlopen")
    def test_falls_back_to_openai_when_anthropic_fails(self, mock_urlopen):
        # First call (Anthropic) raises HTTP error; second call (OpenAI) succeeds
        mock_urlopen.side_effect = [
            urllib.error.HTTPError(None, 500, "err", {}, None),
            _mock_response(OPENAI_RESPONSE),
        ]
        diagnosis, provider = ask_with_fallback("diagnose")
        self.assertIn("openai", provider)
        self.assertEqual(diagnosis, "Root cause: bad config")

    @patch.dict(os.environ, {"GEMINI_API_KEY": "gk-g"}, clear=True)
    @patch("urllib.request.urlopen")
    def test_gemini_only_when_others_absent(self, mock_urlopen):
        mock_urlopen.return_value = _mock_response(GEMINI_RESPONSE)
        diagnosis, provider = ask_with_fallback("diagnose")
        self.assertIn("gemini", provider)
        self.assertEqual(diagnosis, "Root cause: disk full")

    @patch.dict(os.environ, {"ANTHROPIC_API_KEY": "sk-a"}, clear=True)
    @patch("urllib.request.urlopen")
    def test_raises_when_only_provider_fails(self, mock_urlopen):
        mock_urlopen.side_effect = urllib.error.HTTPError(None, 500, "err", {}, None)
        with self.assertRaises(DiagnosisUnavailable):
            ask_with_fallback("diagnose")
```

- [ ] **Step 2: Run to confirm new tests fail**

```bash
cd scripts && uv run pytest tests/test_providers.py::TestFallbackChain -v 2>&1 | tail -10
```

Expected: `FAILED` — OpenAI/Gemini not implemented yet.

- [ ] **Step 3: Add `_ask_openai` and `_ask_gemini` to providers.py, update fallback list**

Add these two functions to `scripts/ci_agent/providers.py` before `ask_with_fallback`:

```python
def _ask_openai(prompt: str) -> str:
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        raise ValueError("no key")
    body = json.dumps({
        "model": "gpt-4.1",
        "max_tokens": MAX_TOKENS,
        "messages": [{"role": "user", "content": prompt}],
    }).encode()
    req = urllib.request.Request(
        "https://api.openai.com/v1/chat/completions",
        data=body,
        headers={
            "Authorization": f"Bearer {key}",
            "content-type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        resp = json.loads(r.read())
    return resp["choices"][0]["message"]["content"]


def _ask_gemini(prompt: str) -> str:
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        raise ValueError("no key")
    body = json.dumps({
        "contents": [{"parts": [{"text": prompt}]}]
    }).encode()
    url = (
        f"https://generativelanguage.googleapis.com/v1beta/models/"
        f"gemini-2.5-flash:generateContent?key={key}"
    )
    req = urllib.request.Request(
        url, data=body,
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        resp = json.loads(r.read())
    return resp["candidates"][0]["content"]["parts"][0]["text"]
```

Update the `providers` list in `ask_with_fallback`:

```python
    providers = [
        ("anthropic/claude-sonnet-4-6", _ask_anthropic),
        ("openai/gpt-4.1", _ask_openai),
        ("google/gemini-2.5-flash", _ask_gemini),
    ]
```

- [ ] **Step 4: Run all provider tests**

```bash
cd scripts && uv run pytest tests/test_providers.py -v
```

Expected: all 5 tests `PASSED`.

- [ ] **Step 5: Commit**

```bash
git add scripts/ci_agent/providers.py scripts/tests/test_providers.py
git commit -m "feat(ci-agent): add providers.py with Anthropic/OpenAI/Gemini fallback chain"
```

---

## Task 4: issues.py — commit status and label

**Files:**
- Create: `scripts/ci_agent/issues.py`
- Create: `scripts/tests/test_issues.py`

- [ ] **Step 1: Write failing tests for `set_commit_status` and `ensure_label_exists`**

Create `scripts/tests/test_issues.py`:

```python
import json
import unittest
from unittest.mock import MagicMock, patch

from ci_agent.issues import ensure_label_exists, set_commit_status


def _mock_ok(body=None) -> MagicMock:
    m = MagicMock()
    m.read.return_value = json.dumps(body or {}).encode()
    m.__enter__ = lambda s: s
    m.__exit__ = MagicMock(return_value=False)
    return m


HEADERS = {"Authorization": "token tok", "Content-Type": "application/json"}
GITEA = "http://gitea:3000"
REPO = "joe/minibox"


class TestSetCommitStatus(unittest.TestCase):
    @patch("urllib.request.urlopen")
    def test_posts_status(self, mock_urlopen):
        mock_urlopen.return_value = _mock_ok()
        set_commit_status(GITEA, REPO, "abc123", "failure", "Diagnosed: bad dep", HEADERS)
        mock_urlopen.assert_called_once()
        req = mock_urlopen.call_args[0][0]
        self.assertIn("/statuses/abc123", req.full_url)
        body = json.loads(req.data)
        self.assertEqual(body["state"], "failure")
        self.assertEqual(body["context"], "ci/agent-diagnosis")

    @patch("urllib.request.urlopen")
    def test_non_fatal_on_http_error(self, mock_urlopen):
        import urllib.error
        mock_urlopen.side_effect = urllib.error.HTTPError(None, 422, "err", {}, None)
        # Should not raise
        set_commit_status(GITEA, REPO, "abc123", "failure", "msg", HEADERS)


class TestEnsureLabelExists(unittest.TestCase):
    @patch("urllib.request.urlopen")
    def test_creates_label_when_absent(self, mock_urlopen):
        # GET returns empty list; POST creates label
        mock_urlopen.side_effect = [_mock_ok([]), _mock_ok({"id": 1})]
        ensure_label_exists(GITEA, REPO, HEADERS)
        self.assertEqual(mock_urlopen.call_count, 2)
        create_req = mock_urlopen.call_args_list[1][0][0]
        body = json.loads(create_req.data)
        self.assertEqual(body["name"], "ci-failure")

    @patch("urllib.request.urlopen")
    def test_skips_create_when_label_exists(self, mock_urlopen):
        mock_urlopen.return_value = _mock_ok([{"name": "ci-failure", "id": 5}])
        ensure_label_exists(GITEA, REPO, HEADERS)
        # Only the GET — no POST
        mock_urlopen.assert_called_once()
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd scripts && uv run pytest tests/test_issues.py -v 2>&1 | head -15
```

Expected: `ImportError` — `issues.py` doesn't exist.

- [ ] **Step 3: Create `scripts/ci_agent/issues.py` with these two functions**

```python
"""Gitea issue lifecycle and commit status for the CI diagnosis agent."""

import json
import urllib.request
import urllib.error


def set_commit_status(
    gitea_url: str, repo: str, sha: str,
    state: str, description: str, headers: dict,
) -> None:
    """Post ci/agent-diagnosis commit status. Non-fatal on failure."""
    body = json.dumps({
        "context": "ci/agent-diagnosis",
        "state": state,
        "description": description[:140],
    }).encode()
    req = urllib.request.Request(
        f"{gitea_url}/api/v1/repos/{repo}/statuses/{sha}",
        data=body, headers=headers, method="POST",
    )
    try:
        with urllib.request.urlopen(req) as r:
            r.read()
    except Exception as e:
        print(f"  (could not post commit status: {e})")


def ensure_label_exists(gitea_url: str, repo: str, headers: dict) -> None:
    """Create 'ci-failure' label (color #e11d48) if absent. Idempotent."""
    req = urllib.request.Request(
        f"{gitea_url}/api/v1/repos/{repo}/labels", headers=headers
    )
    with urllib.request.urlopen(req) as r:
        labels = json.loads(r.read())
    if any(lbl["name"] == "ci-failure" for lbl in labels):
        return
    body = json.dumps({"name": "ci-failure", "color": "#e11d48"}).encode()
    req = urllib.request.Request(
        f"{gitea_url}/api/v1/repos/{repo}/labels",
        data=body, headers=headers, method="POST",
    )
    with urllib.request.urlopen(req) as r:
        r.read()
```

- [ ] **Step 4: Run tests**

```bash
cd scripts && uv run pytest tests/test_issues.py::TestSetCommitStatus tests/test_issues.py::TestEnsureLabelExists -v
```

Expected: all 4 tests `PASSED`.

---

## Task 5: issues.py — find, create, comment

**Files:**
- Modify: `scripts/ci_agent/issues.py`
- Modify: `scripts/tests/test_issues.py`

- [ ] **Step 1: Add tests for `find_issue_for_commit`, `create_issue`, `add_comment`**

Append to `scripts/tests/test_issues.py`:

```python
from ci_agent.issues import add_comment, create_issue, find_issue_for_commit

SHA = "abc123def456abc123def456abc123def456abc123"


class TestFindIssueForCommit(unittest.TestCase):
    @patch("urllib.request.urlopen")
    def test_returns_none_when_no_issues(self, mock_urlopen):
        mock_urlopen.return_value = _mock_ok([])
        result = find_issue_for_commit(GITEA, REPO, SHA, HEADERS)
        self.assertIsNone(result)

    @patch("urllib.request.urlopen")
    def test_finds_issue_by_sha_marker(self, mock_urlopen):
        issues = [{"number": 42, "body": f"some text\n<!-- sha: {SHA} -->"}]
        mock_urlopen.return_value = _mock_ok(issues)
        result = find_issue_for_commit(GITEA, REPO, SHA, HEADERS)
        self.assertEqual(result, 42)

    @patch("urllib.request.urlopen")
    def test_paginates_until_empty(self, mock_urlopen):
        # Page 1: issue with wrong SHA; page 2: empty
        page1 = [{"number": 10, "body": "<!-- sha: deadbeef -->"}]
        mock_urlopen.side_effect = [_mock_ok(page1), _mock_ok([])]
        result = find_issue_for_commit(GITEA, REPO, SHA, HEADERS)
        self.assertIsNone(result)
        self.assertEqual(mock_urlopen.call_count, 2)


class TestCreateIssue(unittest.TestCase):
    @patch("urllib.request.urlopen")
    def test_creates_issue_and_returns_number(self, mock_urlopen):
        mock_urlopen.return_value = _mock_ok({"number": 7})
        number = create_issue(
            GITEA, REPO, SHA, "Root cause: bad dep",
            "anthropic/claude-sonnet-4-6", ["lint"], "99", HEADERS,
        )
        self.assertEqual(number, 7)
        req = mock_urlopen.call_args[0][0]
        body = json.loads(req.data)
        self.assertIn(SHA[:8], body["title"])
        self.assertIn(f"<!-- sha: {SHA} -->", body["body"])
        self.assertIn("ci-failure", body["labels"])

    @patch("urllib.request.urlopen")
    def test_body_contains_run_link(self, mock_urlopen):
        mock_urlopen.return_value = _mock_ok({"number": 8})
        create_issue(
            GITEA, REPO, SHA, "diagnosis",
            "anthropic/claude-sonnet-4-6", ["lint"], "42", HEADERS,
        )
        body = json.loads(mock_urlopen.call_args[0][0].data)
        self.assertIn("42", body["body"])  # run ID in body


class TestAddComment(unittest.TestCase):
    @patch("urllib.request.urlopen")
    def test_posts_comment(self, mock_urlopen):
        mock_urlopen.return_value = _mock_ok({"id": 1})
        add_comment(GITEA, REPO, 42, "Re-run diagnosis", "openai/gpt-4.1", HEADERS)
        req = mock_urlopen.call_args[0][0]
        self.assertIn("/issues/42/comments", req.full_url)
        body = json.loads(req.data)
        self.assertIn("Re-run diagnosis", body["body"])
        self.assertIn("openai/gpt-4.1", body["body"])
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd scripts && uv run pytest tests/test_issues.py -v 2>&1 | tail -15
```

Expected: `ImportError` for `find_issue_for_commit`, `create_issue`, `add_comment`.

- [ ] **Step 3: Add the three functions to `issues.py`**

```python
def find_issue_for_commit(
    gitea_url: str, repo: str, sha: str, headers: dict,
) -> int | None:
    """Return open issue number for this commit SHA, or None.
    Paginates through all open ci-failure issues until match or exhausted."""
    page = 1
    marker = f"<!-- sha: {sha} -->"
    while True:
        url = (
            f"{gitea_url}/api/v1/repos/{repo}/issues"
            f"?state=open&type=issues&labels=ci-failure&limit=50&page={page}"
        )
        req = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(req) as r:
            issues = json.loads(r.read())
        if not issues:
            return None
        for issue in issues:
            if marker in (issue.get("body") or ""):
                return issue["number"]
        page += 1


def create_issue(
    gitea_url: str, repo: str, sha: str, diagnosis: str,
    provider: str, failed_jobs: list[str], run_id: str, headers: dict,
) -> int:
    """Open new issue. Returns issue number."""
    title = f"CI failure: {sha[:8]} — {', '.join(failed_jobs)}"
    run_url = f"{gitea_url}/{repo}/actions/runs/{run_id}"
    body = (
        f"## CI Failure Diagnosis\n\n"
        f"**Jobs:** {', '.join(failed_jobs)}\n"
        f"**Provider:** {provider}\n"
        f"**Commit:** {sha}\n"
        f"**Run:** [{run_id}]({run_url})\n\n"
        f"{diagnosis}\n\n"
        f"---\n"
        f"*Diagnosed by ci-agent. Full logs in the run linked above.*\n"
        f"<!-- sha: {sha} -->"
    )
    data = json.dumps({"title": title, "body": body, "labels": ["ci-failure"]}).encode()
    req = urllib.request.Request(
        f"{gitea_url}/api/v1/repos/{repo}/issues",
        data=data, headers=headers, method="POST",
    )
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())["number"]


def add_comment(
    gitea_url: str, repo: str, issue_number: int,
    diagnosis: str, provider: str, headers: dict,
) -> None:
    """Append re-run diagnosis as a comment."""
    body = f"## Re-run Diagnosis\n\n**Provider:** {provider}\n\n{diagnosis}"
    data = json.dumps({"body": body}).encode()
    req = urllib.request.Request(
        f"{gitea_url}/api/v1/repos/{repo}/issues/{issue_number}/comments",
        data=data, headers=headers, method="POST",
    )
    with urllib.request.urlopen(req) as r:
        r.read()
```

- [ ] **Step 4: Run all issues tests**

```bash
cd scripts && uv run pytest tests/test_issues.py -v
```

Expected: all 11 tests `PASSED`.

- [ ] **Step 5: Commit**

```bash
git add scripts/ci_agent/issues.py scripts/tests/test_issues.py
git commit -m "feat(ci-agent): add issues.py — Gitea issue lifecycle with SHA deduplication"
```

---

## Task 6: `__main__.py` — orchestrator

**Files:**
- Create: `scripts/ci_agent/__main__.py`
- Create: `scripts/tests/test_main.py`

- [ ] **Step 1: Write integration test for the main flow**

Create `scripts/tests/test_main.py`:

```python
import os
import sys
import unittest
from unittest.mock import patch


def _env():
    return {
        "GITEA_URL": "http://gitea:3000",
        "CI_AGENT_TOKEN": "tok",
        "REPO": "joe/minibox",
        "RUN_ID": "99",
        "COMMIT_SHA": "abc123def456abc123def456abc123def456abc123",
        "ANTHROPIC_API_KEY": "sk-a",
    }


class TestMainFlow(unittest.TestCase):
    @patch("ci_agent.__main__.find_issue_for_commit", return_value=None)
    @patch("ci_agent.__main__.create_issue", return_value=7)
    @patch("ci_agent.__main__.ensure_label_exists")
    @patch("ci_agent.__main__.set_commit_status")
    @patch("ci_agent.__main__.ask_with_fallback",
           return_value=("diagnosis text", "anthropic/claude-sonnet-4-6"))
    @patch("ci_agent.__main__.fetch_job_log", return_value="log output")
    @patch("ci_agent.__main__.gitea_get",
           return_value={"jobs": [{"id": 1, "name": "lint", "conclusion": "failure"}]})
    @patch.dict(os.environ, _env(), clear=False)
    def test_creates_issue_on_first_failure(
        self, mock_get, mock_log, mock_ask, mock_status,
        mock_label, mock_create, mock_find,
    ):
        from ci_agent.__main__ import main
        main()
        mock_create.assert_called_once()

    @patch("ci_agent.__main__.find_issue_for_commit", return_value=42)
    @patch("ci_agent.__main__.add_comment")
    @patch("ci_agent.__main__.ensure_label_exists")
    @patch("ci_agent.__main__.set_commit_status")
    @patch("ci_agent.__main__.ask_with_fallback",
           return_value=("re-diagnosis", "openai/gpt-4.1"))
    @patch("ci_agent.__main__.fetch_job_log", return_value="log")
    @patch("ci_agent.__main__.gitea_get",
           return_value={"jobs": [{"id": 2, "name": "lint", "conclusion": "failure"}]})
    @patch.dict(os.environ, _env(), clear=False)
    def test_comments_on_existing_issue(
        self, mock_get, mock_log, mock_ask, mock_status,
        mock_label, mock_comment, mock_find,
    ):
        from ci_agent.__main__ import main
        main()
        mock_comment.assert_called_once()

    @patch("ci_agent.__main__.gitea_get",
           return_value={"jobs": [{"id": 1, "name": "lint", "conclusion": "success"}]})
    @patch.dict(os.environ, _env(), clear=False)
    def test_exits_zero_when_no_failures(self, mock_get):
        from ci_agent.__main__ import main
        with self.assertRaises(SystemExit) as cm:
            main()
        self.assertEqual(cm.exception.code, 0)

    @patch.dict(os.environ, {}, clear=True)
    def test_exits_one_on_missing_required_env(self):
        from ci_agent.__main__ import main
        with self.assertRaises(SystemExit) as cm:
            main()
        self.assertEqual(cm.exception.code, 1)
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd scripts && uv run pytest tests/test_main.py -v 2>&1 | head -15
```

Expected: `ImportError` — `__main__.py` doesn't exist.

- [ ] **Step 3: Create `scripts/ci_agent/__main__.py`**

```python
"""CI failure diagnosis agent — orchestrates providers and Gitea issue creation."""

import json
import os
import sys
import urllib.request
import urllib.error

from .providers import DiagnosisUnavailable, ask_with_fallback
from .issues import (
    add_comment,
    create_issue,
    ensure_label_exists,
    find_issue_for_commit,
    set_commit_status,
)

MAX_LOG_BYTES = 30_000


def _require_env(key: str) -> str:
    val = os.environ.get(key)
    if not val:
        print(f"ERROR: required env var {key} is not set", file=sys.stderr)
        sys.exit(1)
    return val


def gitea_get(path: str, gitea_url: str, headers: dict) -> dict:
    req = urllib.request.Request(f"{gitea_url}/api/v1{path}", headers=headers)
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


def fetch_job_log(job_id: int, gitea_url: str, repo: str, headers: dict) -> str:
    url = f"{gitea_url}/api/v1/repos/{repo}/actions/jobs/{job_id}/logs"
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        return f"(log unavailable: {e})"
    text = raw.decode("utf-8", errors="replace")
    if len(text) > MAX_LOG_BYTES:
        text = "...[truncated]...\n" + text[-MAX_LOG_BYTES:]
    return text


def main() -> None:
    gitea_url = _require_env("GITEA_URL")
    token = _require_env("CI_AGENT_TOKEN")
    repo = _require_env("REPO")
    run_id = _require_env("RUN_ID")
    sha = _require_env("COMMIT_SHA")

    headers = {"Authorization": f"token {token}", "Content-Type": "application/json"}

    print(f"=== CI Diagnosis Agent — run {run_id} ===")

    jobs_data = gitea_get(
        f"/repos/{repo}/actions/runs/{run_id}/jobs?limit=50", gitea_url, headers
    )
    jobs = jobs_data.get("jobs", [])
    failed = [j for j in jobs if j.get("conclusion") == "failure"]
    if not failed:
        print("No failed jobs found — nothing to diagnose.")
        sys.exit(0)

    print(f"Failed jobs: {[j['name'] for j in failed]}")

    sections = []
    for job in failed:
        job_name = job.get("name", f"job-{job['id']}")
        print(f"  Fetching logs: {job_name} (id={job['id']})")
        log = fetch_job_log(job["id"], gitea_url, repo, headers)
        sections.append(f"### Job: {job_name}\n```\n{log}\n```")

    logs_text = "\n\n".join(sections)
    prompt = (
        "You are a Rust/CI expert. The following CI jobs failed in the minibox project "
        "(a Linux container runtime written in Rust, using cargo xtask for all CI tasks).\n\n"
        f"{logs_text}\n\n"
        "Analyze the failures and provide:\n"
        "1. **Root cause** — what exactly failed and why\n"
        "2. **Fix** — the minimal code or config change needed\n"
        "3. **Confidence** — how certain you are (high/medium/low)\n\n"
        "Be concise. Focus on actionable fixes."
    )

    print("\nAsking LLM for diagnosis...")
    set_commit_status(gitea_url, repo, sha, "pending", "CI diagnosis in progress...", headers)

    try:
        diagnosis, provider = ask_with_fallback(prompt)
    except DiagnosisUnavailable as e:
        print(f"All LLM providers failed: {e}")
        diagnosis = f"All LLM providers failed — raw logs attached.\n\n{logs_text}"
        provider = "none"

    print("\n" + "=" * 60)
    print("DIAGNOSIS")
    print("=" * 60)
    print(diagnosis)
    print("=" * 60)

    first_line = diagnosis.split("\n")[0][:120]
    state = "failure" if provider != "none" else "error"
    set_commit_status(gitea_url, repo, sha, state, f"Diagnosed: {first_line}", headers)

    failed_job_names = [j.get("name", f"job-{j['id']}") for j in failed]
    try:
        ensure_label_exists(gitea_url, repo, headers)
        existing = find_issue_for_commit(gitea_url, repo, sha, headers)
        if existing:
            print(f"Appending to existing issue #{existing}")
            add_comment(gitea_url, repo, existing, diagnosis, provider, headers)
        else:
            number = create_issue(
                gitea_url, repo, sha, diagnosis,
                provider, failed_job_names, run_id, headers,
            )
            print(f"Opened issue #{number}")
    except Exception as e:
        print(f"Warning: could not create/update Gitea issue: {e}")


if __name__ == "__main__":
    main()
```

- [ ] **Step 4: Run all tests**

```bash
cd scripts && uv run pytest tests/ -v
```

Expected: all ~18 tests `PASSED`.

- [ ] **Step 5: Commit**

```bash
git add scripts/ci_agent/__main__.py scripts/tests/test_main.py
git commit -m "feat(ci-agent): add orchestrator __main__.py"
```

---

## Task 7: Migration — replace old script, update workflow

**Files:**
- Delete: `scripts/ci_agent.py`
- Modify: `.gitea/workflows/ci.yml`

- [ ] **Step 1: Run full test suite one final time before deleting anything**

```bash
cd scripts && uv run pytest tests/ -v
```

Expected: all tests `PASSED`.

- [ ] **Step 2: Delete the old script**

```bash
git rm scripts/ci_agent.py
```

- [ ] **Step 3: Update `.gitea/workflows/ci.yml`**

Replace the `diagnose` job's `env:` and `run:` lines so the final job looks like:

```yaml
  diagnose:
    name: Diagnose Failures
    runs-on: ubuntu-latest
    needs: [lint]
    if: failure()
    steps:
      - uses: actions/checkout@v4
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

- [ ] **Step 4: Add `OPENAI_API_KEY` and `GEMINI_API_KEY` as secrets in Gitea (manual)**

Navigate to `http://100.105.75.7:3000/joe/minibox/settings/secrets` and add:
- `OPENAI_API_KEY` — your OpenAI API key
- `GEMINI_API_KEY` — your Google Gemini API key

The existing `ANTHROPIC_API_KEY` secret stays as-is.

- [ ] **Step 5: Run tests one final time**

```bash
cd scripts && uv run pytest tests/ -v
```

Expected: all tests `PASSED`.

- [ ] **Step 6: Commit**

```bash
git add .gitea/workflows/ci.yml
git commit -m "feat(ci-agent): migrate to package — provider fallback + Gitea issue creation"
```

- [ ] **Step 7: Push and verify**

```bash
git push origin main
```

Watch `http://100.105.75.7:3000/joe/minibox/actions` — confirm the `diagnose` job still appears on failure. On the next lint failure, a `ci-failure` labelled issue should be opened on the repo at `http://100.105.75.7:3000/joe/minibox/issues`.
