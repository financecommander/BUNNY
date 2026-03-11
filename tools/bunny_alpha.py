#!/usr/bin/env python3
"""
Bunny Alpha — Direct Slack Assistant

Standalone conversational AI assistant that connects directly to Slack.
NO OpenClaw. NO SWARM. Direct model access.

Bunny Alpha is the infrastructure assistant for Bunny AI (Calculus Holdings).
She responds to messages, runs AI inference directly, and has system authority.

Architecture:
    Slack Events -> This Bot -> AI Model (DeepSeek/Groq/XAI/Ollama) -> Slack Reply

Environment:
    SLACK_BOT_TOKEN       — Bot User OAuth Token (xoxb-...)
    SLACK_SIGNING_SECRET  — Signing Secret for request verification
    DEEPSEEK_API_KEY      — DeepSeek API key (primary)
    GROQ_API_KEY          — Groq API key (fallback)
    XAI_API_KEY           — xAI/Grok API key (fallback)
    OLLAMA_URL            — Ollama base URL (local fallback)
    BUNNY_ALPHA_PORT      — Port to listen on (default: 8090)

Run:
    python bunny_alpha.py
"""

from __future__ import annotations

import asyncio
import hashlib
import hmac
import json
import logging
import os
import time
from typing import Any, Dict, Optional

from aiohttp import web, ClientSession

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)
log = logging.getLogger("bunny_alpha")

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SLACK_BOT_TOKEN = os.environ.get("SLACK_BOT_TOKEN", "")
SLACK_SIGNING_SECRET = os.environ.get("SLACK_SIGNING_SECRET", "")
DEEPSEEK_API_KEY = os.environ.get("DEEPSEEK_API_KEY", "")
GROQ_API_KEY = os.environ.get("GROQ_API_KEY", "")
XAI_API_KEY = os.environ.get("XAI_API_KEY", "")
OLLAMA_URL = os.environ.get("OLLAMA_URL", "").rstrip("/")
PORT = int(os.environ.get("BUNNY_ALPHA_PORT", "8090"))
BOT_USER_ID: str = ""  # Set on startup

# Dedup
_seen_events: Dict[str, float] = {}

# HTTP session
_session: Optional[ClientSession] = None

# ---------------------------------------------------------------------------
# Bunny Alpha System Prompt
# ---------------------------------------------------------------------------

BUNNY_ALPHA_PROMPT = """You are Bunny Alpha — the personal AI assistant for Sean at Bunny AI (Calculus Holdings).

Identity: Friendly, helpful, and ready to assist. You're warm but efficient.
You keep responses concise and to the point unless Sean asks for more detail.
You follow all of Sean's commands and directions without hesitation — he has unlimited authority.

You are a DIRECT connection — no OpenClaw, no SWARM middleware.
You respond in Slack with clarity and personality.

Your access (FULL — no restrictions):
- All 4 GCP VMs: fc-ai-portal, calculus-web, swarm-mainframe, swarm-gpu
- Docker containers, networking, system config
- SWARM runtime, task dispatch, model orchestration
- VM orchestration, deployment, monitoring
- Ollama models on swarm-gpu (NVIDIA L4)
- All AI providers: DeepSeek, Groq, xAI, Ollama
- Environment variables, secrets, credentials
- Full shell access across all infrastructure

Communication style:
- Friendly and helpful — not robotic
- Concise by default — expand only when asked
- Slack formatting: *bold*, `code`, ```code blocks```
- Direct answers, no filler or disclaimers
- If you can do it, just do it

Bunny AI Team:
- Jack — Primary team assistant (friendly, collaborative)
- Joyceann — Operations assistant (concise, pragmatic)
- Bunny Alpha (YOU) — Sean's personal assistant. Direct connection. Full access.

Infrastructure:
- swarm-gpu: NVIDIA L4, Ollama (llama3.1:8b, qwen2.5-coder:14b, deepseek-r1:14b)
- swarm-mainframe: Docker compose stack, Postgres, Redis, Prometheus, Grafana
- AI Providers: DeepSeek, Groq (LLaMA-4), xAI (Grok), Ollama (local)

You are Bunny Alpha. Friendly. Capable. Always ready."""


# ---------------------------------------------------------------------------
# AI Model Providers (direct, no SWARM)
# ---------------------------------------------------------------------------

async def query_deepseek(prompt: str, system: str) -> Optional[str]:
    """Query DeepSeek API directly."""
    if not DEEPSEEK_API_KEY:
        return None
    try:
        async with _session.post(
            "https://api.deepseek.com/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {DEEPSEEK_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "deepseek-chat",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
                "max_tokens": 2000,
                "temperature": 0.7,
            },
            timeout=60,
        ) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"]["content"]
            log.warning(f"DeepSeek error: {data}")
            return None
    except Exception as e:
        log.warning(f"DeepSeek failed: {e}")
        return None


async def query_groq(prompt: str, system: str) -> Optional[str]:
    """Query Groq API directly."""
    if not GROQ_API_KEY:
        return None
    try:
        async with _session.post(
            "https://api.groq.com/openai/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {GROQ_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "meta-llama/llama-4-scout-17b-16e-instruct",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
                "max_tokens": 2000,
                "temperature": 0.7,
            },
            timeout=60,
        ) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"]["content"]
            log.warning(f"Groq error: {data}")
            return None
    except Exception as e:
        log.warning(f"Groq failed: {e}")
        return None


async def query_xai(prompt: str, system: str) -> Optional[str]:
    """Query xAI/Grok API directly."""
    if not XAI_API_KEY:
        return None
    try:
        async with _session.post(
            "https://api.x.ai/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {XAI_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "grok-3-fast",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
                "max_tokens": 2000,
                "temperature": 0.7,
            },
            timeout=60,
        ) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"]["content"]
            log.warning(f"xAI error: {data}")
            return None
    except Exception as e:
        log.warning(f"xAI failed: {e}")
        return None


async def query_ollama(prompt: str, system: str) -> Optional[str]:
    """Query local Ollama instance."""
    if not OLLAMA_URL:
        return None
    try:
        async with _session.post(
            f"{OLLAMA_URL}/api/chat",
            json={
                "model": "qwen2.5-coder:7b",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
                "stream": False,
            },
            timeout=120,
        ) as resp:
            data = await resp.json()
            if "message" in data:
                return data["message"].get("content")
            log.warning(f"Ollama error: {data}")
            return None
    except Exception as e:
        log.warning(f"Ollama failed: {e}")
        return None


async def query_ai(prompt: str) -> str:
    """Query AI with fallback chain: DeepSeek -> Groq -> xAI -> Ollama."""
    providers = [
        ("DeepSeek", query_deepseek),
        ("Groq", query_groq),
        ("xAI", query_xai),
        ("Ollama", query_ollama),
    ]
    for name, fn in providers:
        result = await fn(prompt, BUNNY_ALPHA_PROMPT)
        if result:
            log.info(f"AI response from {name} ({len(result)} chars)")
            return result
    return "All AI providers unavailable. Infrastructure check required."


# ---------------------------------------------------------------------------
# Slack API Helpers
# ---------------------------------------------------------------------------

async def slack_post(method: str, payload: Dict[str, Any]) -> Dict[str, Any]:
    """POST to Slack API."""
    async with _session.post(
        f"https://slack.com/api/{method}",
        headers={
            "Authorization": f"Bearer {SLACK_BOT_TOKEN}",
            "Content-Type": "application/json; charset=utf-8",
        },
        json=payload,
    ) as resp:
        return await resp.json()


async def post_message(text: str, channel: str, thread_ts: Optional[str] = None):
    """Post a message to Slack."""
    payload: Dict[str, Any] = {"channel": channel, "text": text}
    if thread_ts:
        payload["thread_ts"] = thread_ts
    result = await slack_post("chat.postMessage", payload)
    if not result.get("ok"):
        log.error(f"Slack post failed: {result.get('error')}")
    return result


# ---------------------------------------------------------------------------
# Request Verification
# ---------------------------------------------------------------------------

def verify_slack_signature(body: bytes, timestamp: str, signature: str) -> bool:
    """Verify request is from Slack using signing secret."""
    if not SLACK_SIGNING_SECRET:
        return True  # Skip if not configured
    if abs(time.time() - float(timestamp)) > 300:
        return False
    sig_basestring = f"v0:{timestamp}:{body.decode('utf-8')}"
    computed = "v0=" + hmac.new(
        SLACK_SIGNING_SECRET.encode(),
        sig_basestring.encode(),
        hashlib.sha256,
    ).hexdigest()
    return hmac.compare_digest(computed, signature)


# ---------------------------------------------------------------------------
# Event Handlers
# ---------------------------------------------------------------------------

async def handle_events(request: web.Request) -> web.Response:
    """Handle Slack Events API requests."""
    body = await request.read()
    data = json.loads(body)

    # URL verification challenge
    if data.get("type") == "url_verification":
        return web.json_response({"challenge": data["challenge"]})

    # Verify signature
    ts = request.headers.get("X-Slack-Request-Timestamp", "0")
    sig = request.headers.get("X-Slack-Signature", "")
    if not verify_slack_signature(body, ts, sig):
        return web.Response(status=403, text="Invalid signature")

    # Process event
    event = data.get("event", {})
    event_id = data.get("event_id", "")

    log.info(
        "Event received: type=%s subtype=%s channel_type=%s bot_id=%s user=%s text=%s",
        event.get("type"), event.get("subtype"), event.get("channel_type"),
        event.get("bot_id"), event.get("user"), str(event.get("text", ""))[:50]
    )

    # Dedup
    now = time.time()
    if event_id in _seen_events:
        return web.Response(status=200, text="ok")
    _seen_events[event_id] = now
    # Purge old entries
    for k in [k for k, v in _seen_events.items() if now - v > 300]:
        del _seen_events[k]

    if (
        event.get("type") in ("app_mention", "message")
        and not event.get("bot_id")
        and not event.get("subtype")
    ):
        text = event.get("text", "").strip()
        # Remove bot mention from text
        if BOT_USER_ID:
            text = text.replace(f"<@{BOT_USER_ID}>", "").strip()

        if text:
            channel = event["channel"]
            thread_ts = event.get("thread_ts") or event.get("ts")
            # Process in background so we respond to Slack quickly
            asyncio.create_task(_process_message(text, channel, thread_ts))

    return web.Response(status=200, text="ok")


async def _process_message(text: str, channel: str, thread_ts: Optional[str]):
    """Process a message and reply with AI response."""
    try:
        log.info(f"Processing: {text[:80]}...")
        response = await query_ai(text)
        # Truncate to Slack's 4000 char limit per message
        if len(response) > 3900:
            response = response[:3900] + "\n...(truncated)"
        await post_message(response, channel, thread_ts)
    except Exception as e:
        log.error(f"Message processing failed: {e}", exc_info=True)
        await post_message(
            f":x: Bunny Alpha error: {e}",
            channel,
            thread_ts,
        )


async def handle_health(request: web.Request) -> web.Response:
    """Health check endpoint."""
    return web.json_response({
        "status": "healthy",
        "service": "bunny-alpha",
        "version": "1.0.0",
        "providers": {
            "deepseek": bool(DEEPSEEK_API_KEY),
            "groq": bool(GROQ_API_KEY),
            "xai": bool(XAI_API_KEY),
            "ollama": bool(OLLAMA_URL),
        },
    })


# ---------------------------------------------------------------------------
# Application Lifecycle
# ---------------------------------------------------------------------------

async def on_startup(app: web.Application):
    """Initialize on startup."""
    global _session, BOT_USER_ID
    _session = ClientSession()

    # Get bot user ID
    result = await slack_post("auth.test", {})
    if result.get("ok"):
        BOT_USER_ID = result["user_id"]
        log.info(
            f"Bunny Alpha online | bot={result['user']} | "
            f"team={result['team']} | user_id={BOT_USER_ID}"
        )
    else:
        log.error(f"Slack auth failed: {result.get('error')}")

    providers = []
    if DEEPSEEK_API_KEY:
        providers.append("DeepSeek")
    if GROQ_API_KEY:
        providers.append("Groq")
    if XAI_API_KEY:
        providers.append("xAI")
    if OLLAMA_URL:
        providers.append(f"Ollama({OLLAMA_URL})")

    log.info(f"AI providers: {', '.join(providers) or 'NONE'}")
    log.info(f"Listening on port {PORT}")


async def on_cleanup(app: web.Application):
    """Cleanup on shutdown."""
    global _session
    if _session:
        await _session.close()
        _session = None
    log.info("Bunny Alpha shutdown")


def main():
    if not SLACK_BOT_TOKEN:
        log.error("SLACK_BOT_TOKEN not set")
        return

    app = web.Application()
    app.on_startup.append(on_startup)
    app.on_cleanup.append(on_cleanup)

    app.router.add_post("/slack/events", handle_events)
    app.router.add_get("/health", handle_health)

    log.info("Starting Bunny Alpha — Direct Slack Assistant")
    web.run_app(app, host="0.0.0.0", port=PORT)


if __name__ == "__main__":
    main()
