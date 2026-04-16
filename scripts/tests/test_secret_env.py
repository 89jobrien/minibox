import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import secret_env


class TestSecretEnv(unittest.TestCase):
    @patch.dict(os.environ, {}, clear=True)
    def test_loads_nearest_env_files_and_local_overrides_base(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text("OPENAI_API_KEY=sk-base\n")
            (root / ".env.local").write_text('OPENAI_API_KEY="sk-local"\n')
            nested = root / "a" / "b"
            nested.mkdir(parents=True)

            loaded = secret_env.load_secret_env(nested)

            self.assertEqual(
                loaded,
                [(root / ".env").resolve(), (root / ".env.local").resolve()],
            )
            self.assertEqual(os.environ["OPENAI_API_KEY"], "sk-local")

    @patch.dict(os.environ, {"OPENAI_API_KEY": "shell-value"}, clear=True)
    def test_existing_env_wins_over_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text("OPENAI_API_KEY=sk-file\n")

            secret_env.load_secret_env(root)

            self.assertEqual(os.environ["OPENAI_API_KEY"], "shell-value")

    @patch.dict(os.environ, {"MINIBOX_SECRETS_FILE": "custom.env"}, clear=True)
    def test_override_file_is_used(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "custom.env").write_text("ANTHROPIC_API_KEY='sk-custom'\n")

            loaded = secret_env.load_secret_env(root)

            self.assertEqual(loaded, [(root / "custom.env").resolve()])
            self.assertEqual(os.environ["ANTHROPIC_API_KEY"], "sk-custom")

    @patch.dict(os.environ, {"OPENAI_API_KEY": "op://vault/item/field"}, clear=True)
    def test_placeholder_provider_env_is_removed(self):
        secret_env.sanitize_provider_env(("OPENAI_API_KEY",))
        self.assertNotIn("OPENAI_API_KEY", os.environ)
