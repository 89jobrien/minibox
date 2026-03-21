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
        "https://generativelanguage.googleapis.com/v1beta/models/"
        "gemini-2.5-flash:generateContent"
    )
    req = urllib.request.Request(
        url, data=body,
        headers={"content-type": "application/json", "x-goog-api-key": key},
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        resp = json.loads(r.read())
    return resp["candidates"][0]["content"]["parts"][0]["text"]


def ask_with_fallback(prompt: str) -> tuple[str, str]:
    """Try Anthropic → OpenAI → Gemini.
    Returns (diagnosis_text, provider_name_used).
    Raises DiagnosisUnavailable if all providers fail or no keys are set."""
    providers = [
        ("anthropic/claude-sonnet-4-6", _ask_anthropic),
        ("openai/gpt-4.1", _ask_openai),
        ("google/gemini-2.5-flash", _ask_gemini),
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
