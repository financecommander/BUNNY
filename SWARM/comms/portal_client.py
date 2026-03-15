"""
AI Portal Client
================
Fetches LLM API keys and makes LLM calls via fc-ai-portal (34.139.78.75).
BUNNY never stores API keys locally — the portal is the single source of truth.

Internal network call: http://fc-ai-portal:8000  (GCP internal)
External fallback:     http://34.139.78.75:8000

Environment:
  PORTAL_URL          — override portal base URL
  PORTAL_INTERNAL_KEY — shared internal service key for machine-to-machine auth
"""

import os
import json
import urllib.request
import urllib.error
from typing import Optional

PORTAL_URL = os.getenv("PORTAL_URL", "http://fc-ai-portal:8000").rstrip("/")
PORTAL_INTERNAL_KEY = os.getenv("PORTAL_INTERNAL_KEY", "")

# Cache keys for the process lifetime (avoid repeated portal calls)
_key_cache: dict[str, str] = {}


def _portal_get(path: str) -> dict:
    """GET from the portal with internal service auth."""
    url = f"{PORTAL_URL}{path}"
    req = urllib.request.Request(url)
    if PORTAL_INTERNAL_KEY:
        req.add_header("X-Internal-Key", PORTAL_INTERNAL_KEY)
    req.add_header("Accept", "application/json")
    with urllib.request.urlopen(req, timeout=5) as resp:
        return json.loads(resp.read().decode())


def get_api_key(provider: str) -> str:
    """
    Fetch an API key from the portal by provider name.
    Providers: anthropic, openai, google, grok, deepseek, mistral, groq

    Returns empty string if not available.
    """
    if provider in _key_cache:
        return _key_cache[provider]

    try:
        data = _portal_get(f"/internal/keys/{provider}")
        key = data.get("key", "")
        if key:
            _key_cache[provider] = key
        return key
    except Exception as e:
        print(f"[portal_client] could not fetch {provider} key: {e}", flush=True)
        # Fall back to local env var if set
        env_map = {
            "anthropic": "ANTHROPIC_API_KEY",
            "openai":    "OPENAI_API_KEY",
            "google":    "GOOGLE_API_KEY",
            "grok":      "XAI_API_KEY",
            "deepseek":  "DEEPSEEK_API_KEY",
            "mistral":   "MISTRAL_API_KEY",
            "groq":      "GROQ_API_KEY",
        }
        return os.getenv(env_map.get(provider, ""), "")


def get_anthropic_key() -> str:
    return get_api_key("anthropic")


def get_openai_key() -> str:
    return get_api_key("openai")
