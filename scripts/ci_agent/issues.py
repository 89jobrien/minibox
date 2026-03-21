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
