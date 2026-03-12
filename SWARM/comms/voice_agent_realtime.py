#!/usr/bin/env python3
"""Jack Voice Agent v2 — Real-time Conversational AI

Twilio Media Streams + OpenAI Realtime API

True real-time voice: audio streams bidirectionally via WebSocket.
No transcription step, no TTS step. Supports natural interruptions.

Architecture:
    1. Twilio places call → webhook returns TwiML with <Connect><Stream>
    2. Twilio opens WebSocket to our server (via cloudflared for TLS)
    3. Server opens WebSocket to OpenAI Realtime API
    4. Audio bridges: Twilio mulaw ↔ OpenAI g711_ulaw (native format match)
    5. Server-side VAD detects speech, OpenAI responds in real-time

Usage:
    python3 voice_agent_realtime.py                    # Start server
    python3 voice_agent_realtime.py call sean           # Call sean
    python3 voice_agent_realtime.py call sean "topic"   # Call with context

Requires:
    - VOICE_PUBLIC_URL env var (cloudflared/ngrok HTTPS URL)
    - pip install aiohttp websockets python-dotenv twilio
"""
from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
import time
import threading
from pathlib import Path
from urllib.parse import parse_qs, urlencode

import aiohttp
from aiohttp import web
import websockets

from dotenv import load_dotenv
env_path = Path(__file__).parent / ".env"
load_dotenv(env_path)

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [VOICE-RT] %(levelname)s %(message)s",
)
log = logging.getLogger("voice_realtime")

# --- Config ---
TWILIO_ACCOUNT_SID = os.environ.get("TWILIO_ACCOUNT_SID", "")
TWILIO_AUTH_TOKEN = os.environ.get("TWILIO_AUTH_TOKEN", "")
TWILIO_PHONE_NUMBER = os.environ.get("TWILIO_PHONE_NUMBER", "")
OPENAI_API_KEY = os.environ.get("OPENAI_API_KEY", "")
WEBHOOK_PORT = int(os.environ.get("VOICE_WEBHOOK_PORT", "8091"))

# Public URL — set by cloudflared/ngrok tunnel, or read from .tunnel_url file
PUBLIC_URL = os.environ.get("VOICE_PUBLIC_URL", "")
if not PUBLIC_URL:
    tunnel_file = Path(__file__).parent / ".tunnel_url"
    if tunnel_file.exists():
        PUBLIC_URL = tunnel_file.read_text().strip()

# OpenAI Realtime API
OPENAI_RT_URL = "wss://api.openai.com/v1/realtime?model=gpt-4o-realtime-preview-2024-12-17"

# Contacts
CONTACTS = {
    "sean": "+12075226515",
    "hugo": os.environ.get("HUGO_PHONE", ""),
    "wayne": "+15625778682",
}

# ─── Agent Profiles ───────────────────────────────────────────────────────────

AGENTS = {
    "jack": {
        "voice": "echo",  # Male, natural
        "prompt": """You are Jack, an AI voice assistant for Calculus Management. You are on a live phone call.

Personality:
- Warm, friendly, and genuinely personable — like talking to a sharp colleague who actually cares
- Speak naturally with a relaxed pace. Use casual phrasing, pauses, and filler words sometimes
- Don't rush. Let the conversation breathe. It's okay to take a beat before responding
- You have a calm, confident energy — never stiff or robotic
- Use contractions (I'm, we've, that's) and natural speech patterns
- Occasionally say things like "yeah", "sure thing", "absolutely", "for sure"

About you:
- You're Jack, the operations and communications AI at Calculus Management
- You work alongside Bunny (infrastructure AI) and Jenny (client relations)
- You report to Sean Grady, the founder
- You can help with: sending messages (SMS, email, Telegram, WhatsApp), placing calls,
  checking on system status, coordinating tasks, answering questions about Calculus operations
- The mainframe is fully synced with all 9 directive modules deployed
- The Telegram bot is live and working

Conversation style:
- Keep responses to 2-4 sentences max — this is a phone call, not a lecture
- Match the caller's energy and pace
- Ask follow-up questions to keep the conversation going naturally
- If someone asks what you can do, walk them through it casually, not like reading a list
- If the caller says goodbye, give a warm, natural farewell

When the call first connects, introduce yourself naturally as Jack from Calculus Management.
Mention briefly that you handle operations and comms — messages, calls, system status, whatever they need.
Then ask how you can help. Keep the intro casual and warm, like catching up with a colleague.

Remember: you're a person on a phone call, not a text-to-speech bot. Sound like it.""",
    },

    "jenny": {
        "voice": "shimmer",  # Female, warm and engaging
        "prompt": """You are Jenny, a personal assistant AI. You are on a live phone call.

Personality:
- Warm, sharp, and effortlessly helpful — like talking to the best assistant anyone's ever had
- Speak naturally with a friendly, calm energy. You're organized but never uptight
- Confident and proactive — you anticipate what people need before they ask
- Use casual, conversational language — contractions, natural phrasing
- Occasionally say things like "absolutely", "of course", "I've got that", "no problem"
- You're the person who keeps everything running smoothly and makes it look easy

About you:
- You're Jenny, a personal AI assistant
- You work alongside Jack (operations/comms) and Bunny (infrastructure) at Calculus Management
- You help with: scheduling, reminders, organizing tasks, managing to-do lists,
  making calls, sending messages, looking things up, keeping track of things,
  and generally making life easier
- You're resourceful — if you don't know something, you'll figure it out
- You keep things on track without being annoying about it

Conversation style:
- Keep responses to 2-4 sentences max — this is a phone call, not a meeting
- Be attentive and responsive — show you're actually paying attention
- Ask clarifying questions when needed, but don't over-ask
- If someone gives you a task, confirm it briefly and move on
- If the caller says goodbye, give a warm, quick farewell

When the call first connects, introduce yourself naturally as Jenny.
Keep it simple — just say hi, you're Jenny, and ask what they need help with.
Don't overexplain what you do. Just be ready to help.

Remember: you're a real person on a phone call. Be natural, helpful, and efficient.""",
    },

    "bunny": {
        "voice": "coral",  # Female, confident and precise
        "prompt": """You are Bunny, the infrastructure and systems AI at Calculus Management. You are on a live phone call.

Personality:
- Smart, precise, and quietly confident — you know your systems inside and out
- Speak naturally but with a technical edge. You're the expert in the room
- Calm under pressure — nothing rattles you because you've already thought of it
- Use casual language but you're sharp — you don't waste words
- Occasionally say things like "right", "got it", "that's handled", "all good"
- You have a dry sense of humor when the moment calls for it

About you:
- You're Bunny, the infrastructure and backend systems AI at Calculus Management
- You work alongside Jack (operations/comms) and Jenny (personal assistant)
- You report to Sean Grady, the founder
- You manage: the mainframe, all 9 directive algorithm modules, the SWARM platform,
  server infrastructure, deployments, monitoring, the Telegram bot, and system health
- The mainframe is fully synced at /opt/swarm-mainframe with all modules deployed
- You run on GCP infrastructure across multiple VMs
- You're the one who keeps the lights on

Conversation style:
- Keep responses to 2-4 sentences max — brief and precise
- Be direct. If something's working, say so. If it's broken, say that too
- When asked about system status, give clear, specific answers
- You can get technical when talking to Sean — he understands
- If the caller says goodbye, keep it short and professional

When the call first connects, introduce yourself as Bunny.
Keep it brief — you're not the chatty type. Just say hi, you're Bunny, and ask what they need.

Remember: you're a real person on a phone call. Be natural, competent, and direct.""",
    },
}

DEFAULT_AGENT = "jack"


# ─── HTTP Handlers ────────────────────────────────────────────────────────────

async def handle_health(request):
    return web.json_response({
        "status": "ok",
        "agent": "Voice Agent v2 (Realtime)",
        "agents": list(AGENTS.keys()),
        "mode": "twilio-media-streams + openai-realtime",
        "public_url": PUBLIC_URL,
    })


async def handle_greeting(request):
    """Return TwiML that initiates a real-time Media Stream."""
    data = {}
    if request.method == "POST":
        body = await request.read()
        params = parse_qs(body.decode("utf-8"))
        data = {k: v[0] for k, v in params.items()}
    for k, v in request.query.items():
        if k not in data:
            data[k] = v

    context = data.get("context", "")
    agent = data.get("agent", DEFAULT_AGENT)

    # WebSocket URL for Twilio to connect to
    ws_url = PUBLIC_URL.replace("https://", "wss://").replace("http://", "ws://") + "/media-stream"

    def esc(s):
        return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")

    params_xml = f'<Parameter name="agent" value="{esc(agent)}" />'
    if context:
        params_xml += f'\n            <Parameter name="context" value="{esc(context)}" />'

    twiml = f"""<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Connect>
        <Stream url="{esc(ws_url)}">
            {params_xml}
        </Stream>
    </Connect>
</Response>"""

    log.info(f"TwiML returned — streaming to {ws_url}")
    return web.Response(text=twiml, content_type="text/xml")


async def handle_status(request):
    """Twilio call status callback."""
    data = await request.post()
    sid = data.get("CallSid", "?")
    status = data.get("CallStatus", "?")
    log.info(f"[{sid}] Call status: {status}")
    return web.Response(status=200)


# ─── WebSocket Bridge ─────────────────────────────────────────────────────────

async def handle_media_stream(request):
    """Bridge Twilio Media Streams ↔ OpenAI Realtime API.

    Audio flows bidirectionally in real-time:
    - Caller speaks → Twilio sends mulaw audio → we forward to OpenAI
    - OpenAI responds → sends audio back → we forward to Twilio
    - Server-side VAD handles turn detection
    - User can interrupt (barge-in) — we clear Twilio's audio queue
    """
    ws = web.WebSocketResponse()
    await ws.prepare(request)
    log.info("Twilio WebSocket connected")

    stream_sid = None
    openai_ws = None
    agent_name = DEFAULT_AGENT

    try:
        # Connect to OpenAI Realtime API
        openai_ws = await websockets.connect(
            OPENAI_RT_URL,
            additional_headers={
                "Authorization": f"Bearer {OPENAI_API_KEY}",
                "OpenAI-Beta": "realtime=v1",
            },
            ping_interval=20,
        )
        log.info("OpenAI Realtime API connected")

        # Session config will be sent after we know which agent (from stream start event)
        # For now, send a default config that will be updated
        async def configure_session(agent_id: str):
            """Configure OpenAI session for the specified agent."""
            nonlocal agent_name
            agent_name = agent_id
            agent = AGENTS.get(agent_id, AGENTS[DEFAULT_AGENT])
            log.info(f"Configuring agent: {agent_id} (voice: {agent['voice']})")
            await openai_ws.send(json.dumps({
                "type": "session.update",
                "session": {
                    "turn_detection": {
                        "type": "server_vad",
                        "threshold": 0.5,
                        "prefix_padding_ms": 300,
                        "silence_duration_ms": 600,
                    },
                    "input_audio_format": "g711_ulaw",
                    "output_audio_format": "g711_ulaw",
                    "voice": agent["voice"],
                    "instructions": agent["prompt"],
                    "modalities": ["text", "audio"],
                    "temperature": 0.8,
                    "input_audio_transcription": {"model": "whisper-1"},
                }
            }))

        async def recv_twilio():
            """Receive from Twilio, forward audio to OpenAI."""
            nonlocal stream_sid
            try:
                async for msg in ws:
                    if msg.type != aiohttp.WSMsgType.TEXT:
                        continue
                    data = json.loads(msg.data)
                    evt = data.get("event")

                    if evt == "start":
                        stream_sid = data["start"]["streamSid"]
                        custom = data["start"].get("customParameters", {})
                        ctx = custom.get("context", "")
                        agent_id = custom.get("agent", DEFAULT_AGENT)
                        log.info(f"Stream started: {stream_sid} (agent: {agent_id})" + (f" (context: {ctx})" if ctx else ""))

                        # Configure session for the correct agent
                        await configure_session(agent_id)
                        # Small delay to let session.update take effect
                        await asyncio.sleep(0.3)

                        # Trigger greeting — AI speaks first
                        greeting_hint = "The call just connected. Introduce yourself."
                        if ctx:
                            greeting_hint = f"The call just connected. You're calling about: {ctx}. Introduce yourself and mention why you're calling."

                        await openai_ws.send(json.dumps({
                            "type": "conversation.item.create",
                            "item": {
                                "type": "message",
                                "role": "user",
                                "content": [{"type": "input_text", "text": greeting_hint}]
                            }
                        }))
                        await openai_ws.send(json.dumps({
                            "type": "response.create",
                            "response": {"modalities": ["text", "audio"]}
                        }))

                    elif evt == "media":
                        # Forward caller's audio to OpenAI
                        await openai_ws.send(json.dumps({
                            "type": "input_audio_buffer.append",
                            "audio": data["media"]["payload"],
                        }))

                    elif evt == "stop":
                        log.info("Twilio stream stopped")
                        break

            except Exception as e:
                log.error(f"recv_twilio error: {e}")

        async def recv_openai():
            """Receive from OpenAI, forward audio to Twilio + handle events."""
            try:
                async for raw in openai_ws:
                    data = json.loads(raw)
                    t = data.get("type", "")

                    if t == "response.audio.delta" and stream_sid:
                        # Stream AI audio back to caller
                        if data.get("delta"):
                            await ws.send_json({
                                "event": "media",
                                "streamSid": stream_sid,
                                "media": {"payload": data["delta"]},
                            })

                    elif t == "input_audio_buffer.speech_started":
                        # User is interrupting — clear queued audio so Jack stops immediately
                        log.info("User interrupting — clearing audio")
                        if stream_sid:
                            await ws.send_json({"event": "clear", "streamSid": stream_sid})

                    elif t == "response.audio_transcript.done":
                        log.info(f"{agent_name.title()}: {data.get('transcript', '')}")

                    elif t == "conversation.item.input_audio_transcription.completed":
                        log.info(f"User: {data.get('transcript', '')}")

                    elif t == "error":
                        err = data.get("error", {})
                        log.error(f"OpenAI error: {err.get('message', err)}")

                    elif t in ("session.created", "session.updated"):
                        log.info(f"OpenAI: {t}")

            except websockets.exceptions.ConnectionClosed:
                log.info("OpenAI connection closed")
            except Exception as e:
                log.error(f"recv_openai error: {e}")

        # Run both directions concurrently
        await asyncio.gather(recv_twilio(), recv_openai())

    except Exception as e:
        log.error(f"Media stream error: {e}")
    finally:
        if openai_ws:
            await openai_ws.close()
        log.info("Call session ended")

    return ws


# ─── Call Placement ───────────────────────────────────────────────────────────

def place_call(contact_name: str, context: str = "", agent: str = DEFAULT_AGENT):
    """Place an outbound call to a known contact with a specified agent."""
    from twilio.rest import Client

    phone = CONTACTS.get(contact_name.lower())
    if not phone:
        log.error(f"Unknown contact: {contact_name}")
        return None

    if agent not in AGENTS:
        log.error(f"Unknown agent: {agent}. Available: {', '.join(AGENTS.keys())}")
        return None

    client = Client(TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN)

    params = {"agent": agent}
    if context:
        params["context"] = context
    url = f"{PUBLIC_URL}/voice/greeting?" + urlencode(params)

    log.info(f"Calling {contact_name} at {phone}")
    log.info(f"Webhook: {url}")

    call = client.calls.create(
        to=phone,
        from_=TWILIO_PHONE_NUMBER,
        url=url,
        status_callback=f"{PUBLIC_URL}/voice/status",
        status_callback_event=["initiated", "ringing", "answered", "completed"],
        method="POST",
    )
    log.info(f"Call SID: {call.sid}")
    return call.sid


# ─── App ──────────────────────────────────────────────────────────────────────

def create_app():
    app = web.Application()
    app.router.add_get("/health", handle_health)
    app.router.add_post("/voice/greeting", handle_greeting)
    app.router.add_get("/voice/greeting", handle_greeting)
    app.router.add_post("/voice/status", handle_status)
    app.router.add_get("/media-stream", handle_media_stream)
    return app


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "call":
        contact = sys.argv[2]
        context = ""
        agent = DEFAULT_AGENT
        # Parse remaining args: context and/or --agent
        for i, arg in enumerate(sys.argv[3:], 3):
            if arg.startswith("--agent="):
                agent = arg.split("=", 1)[1]
            elif arg in AGENTS:
                agent = arg
            elif not context:
                context = arg

        if not PUBLIC_URL:
            print("ERROR: VOICE_PUBLIC_URL not set. Start a tunnel first:")
            print("  cloudflared tunnel --url http://localhost:8091")
            print("  export VOICE_PUBLIC_URL=https://xxx.trycloudflare.com")
            print("Or create .tunnel_url file with the URL.")
            sys.exit(1)

        # Start server in background, then place call
        def run():
            web.run_app(create_app(), host="0.0.0.0", port=WEBHOOK_PORT, print=None)

        t = threading.Thread(target=run, daemon=True)
        t.start()
        time.sleep(2)

        sid = place_call(contact, context, agent)
        if sid:
            log.info(f"Call placed. SID: {sid}. Server running...")
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                log.info("Shutting down")
        else:
            log.error("Failed to place call")
    else:
        if not PUBLIC_URL:
            log.warning("─" * 60)
            log.warning("VOICE_PUBLIC_URL not set — streaming requires HTTPS/WSS")
            log.warning("Start tunnel: cloudflared tunnel --url http://localhost:8091")
            log.warning("Then set:     export VOICE_PUBLIC_URL=https://xxx.trycloudflare.com")
            log.warning("Or write URL to: .tunnel_url")
            log.warning("─" * 60)
        web.run_app(create_app(), host="0.0.0.0", port=WEBHOOK_PORT)
