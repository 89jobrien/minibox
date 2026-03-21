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
