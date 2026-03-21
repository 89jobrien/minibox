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
