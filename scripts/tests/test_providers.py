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
