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

    @patch("ci_agent.__main__.find_issue_for_commit", return_value=None)
    @patch("ci_agent.__main__.create_issue", return_value=7)
    @patch("ci_agent.__main__.ensure_label_exists")
    @patch("ci_agent.__main__.set_commit_status")
    @patch("ci_agent.__main__.ask_with_fallback",
           side_effect=__import__("ci_agent.providers", fromlist=["DiagnosisUnavailable"]).DiagnosisUnavailable("all providers failed"))
    @patch("ci_agent.__main__.fetch_job_log", return_value="log output")
    @patch("ci_agent.__main__.gitea_get",
           return_value={"jobs": [{"id": 1, "name": "lint", "conclusion": "failure"}]})
    @patch.dict(os.environ, _env(), clear=False)
    def test_fallback_creates_issue_with_no_provider(
        self, mock_get, mock_log, mock_ask, mock_status,
        mock_label, mock_create, mock_find,
    ):
        from ci_agent.__main__ import main
        main()
        mock_create.assert_called_once()
        args, _kwargs = mock_create.call_args
        self.assertEqual(args[4], "none")
