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
