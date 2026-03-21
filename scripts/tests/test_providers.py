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
