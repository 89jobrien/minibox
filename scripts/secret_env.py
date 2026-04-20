"""Shared dotenv-style secret loading for local minibox tooling."""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
from pathlib import Path

DEFAULT_FILENAMES = (".env", ".env.local")
PROVIDER_KEYS = ("ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY")
PLACEHOLDER_PREFIXES = ("op://", "bw:", "keyring:", "dotenv:")


def sanitize_provider_env(keys: tuple[str, ...] = PROVIDER_KEYS) -> None:
    for key in keys:
        value = os.environ.get(key, "")
        if any(value.startswith(prefix) for prefix in PLACEHOLDER_PREFIXES):
            os.environ.pop(key, None)


def candidate_files(start_dir: Path | None = None) -> list[Path]:
    override = os.environ.get("MINIBOX_SECRETS_FILE", "").strip()
    if override:
        return [_resolve_path(Path(override), start_dir or Path.cwd())]

    env_dir = _find_env_dir(start_dir or Path.cwd())
    if env_dir is None:
        return []

    return [path for name in DEFAULT_FILENAMES if (path := env_dir / name).is_file()]


def load_secret_env(start_dir: Path | None = None) -> list[Path]:
    original_env = set(os.environ)
    loaded: list[Path] = []

    for path in candidate_files(start_dir):
        for key, value in parse_dotenv_file(path).items():
            if key not in original_env:
                os.environ[key] = value
        loaded.append(path)

    return loaded


def parse_dotenv_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line_no, line in enumerate(path.read_text().splitlines(), start=1):
        parsed = parse_dotenv_line(line)
        if parsed is None:
            continue
        key, value = parsed
        if not key:
            raise ValueError(f"{path}:{line_no}: empty dotenv key")
        values[key] = value
    return values


def parse_dotenv_line(line: str) -> tuple[str, str] | None:
    stripped = line.strip()
    if not stripped or stripped.startswith("#"):
        return None
    if stripped.startswith("export "):
        stripped = stripped[len("export ") :].lstrip()
    if "=" not in stripped:
        raise ValueError(f"expected KEY=VALUE, got {line!r}")

    key, raw_value = stripped.split("=", 1)
    key = key.strip()
    value = _parse_value(raw_value.strip())
    return key, value


def _parse_value(raw: str) -> str:
    if len(raw) >= 2 and raw[0] == raw[-1] == '"':
        return bytes(raw[1:-1], "utf-8").decode("unicode_escape")
    if len(raw) >= 2 and raw[0] == raw[-1] == "'":
        return raw[1:-1]
    if " #" in raw:
        raw = raw.split(" #", 1)[0]
    return raw.rstrip()


def _find_env_dir(start_dir: Path) -> Path | None:
    current = start_dir.resolve()
    for directory in (current, *current.parents):
        if any((directory / name).is_file() for name in DEFAULT_FILENAMES):
            return directory
    return None


def _resolve_path(path: Path, start_dir: Path) -> Path:
    return path if path.is_absolute() else (start_dir / path).resolve()


def _cmd_probe_any(keys: list[str]) -> int:
    sanitize_provider_env(tuple(keys))
    load_secret_env()
    return 0 if any(os.environ.get(key) for key in keys) else 1


def _cmd_export_sh() -> int:
    sanitize_provider_env()
    for path in load_secret_env():
        print(f"# loaded {path}")
    for key in PROVIDER_KEYS:
        value = os.environ.get(key)
        if value:
            print(f"export {key}={shlex.quote(value)}")
    return 0


def _cmd_exec(command: list[str]) -> int:
    sanitize_provider_env()
    load_secret_env()
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise SystemExit("secret_env exec requires a command")
    completed = subprocess.run(command)
    return completed.returncode


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    probe_any = sub.add_parser("probe-any", help="succeed if any listed keys are available")
    probe_any.add_argument("keys", nargs="+")

    sub.add_parser("export-sh", help="print shell exports for loaded provider keys")

    exec_parser = sub.add_parser("exec", help="run a command after loading local secrets")
    exec_parser.add_argument("command_args", nargs=argparse.REMAINDER)

    args = parser.parse_args(argv)
    if args.command == "probe-any":
        return _cmd_probe_any(args.keys)
    if args.command == "export-sh":
        return _cmd_export_sh()
    if args.command == "exec":
        return _cmd_exec(args.command_args)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
