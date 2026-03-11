#!/usr/bin/env python3
"""Jack Voice Agent — Interactive AI Phone Calls via Twilio + OpenAI.

Runs a webhook server that Twilio calls back to during voice calls.
Uses <Gather input="speech"> for real-time conversation with OpenAI.

Architecture:
    1. Twilio places outbound call with webhook URL
    2. Webhook returns TwiML with greeting + <Gather input="speech">
    3. User speaks → Twilio transcribes → POSTs to webhook
    4. Webhook sends transcript to OpenAI → returns <Say> + <Gather>
    5. Loop continues until user says "goodbye" or hangs up

Usage:
    python3 voice_agent.py                   # Start webhook server
    python3 voice_agent.py call sean         # Start server + call sean
    python3 voice_agent.py call sean "topic" # Start server + call with context
"""
from __future__ import annotations

import json
import logging
import os
import sys
import threading
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from pathlib import Path
from urllib.parse import parse_qs, urlencode

# Load env
from dotenv import load_dotenv
env_path = Path(__file__).parent / ".env"
load_dotenv(env_path)

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [VOICE] %(levelname)s %(message)s",
)
log = logging.getLogger("voice_agent")

# Config
TWILIO_ACCOUNT_SID = os.environ.get("TWILIO_ACCOUNT_SID", "")
TWILIO_AUTH_TOKEN = os.environ.get("TWILIO_AUTH_TOKEN", "")
TWILIO_PHONE_NUMBER = os.environ.get("TWILIO_PHONE_NUMBER", "")
OPENAI_API_KEY = os.environ.get("OPENAI_API_KEY", "")
WEBHOOK_PORT = int(os.environ.get("VOICE_WEBHOOK_PORT", "8091"))
PUBLIC_IP = os.environ.get("PUBLIC_IP", "34.139.78.75")
WEBHOOK_BASE = f"http://{PUBLIC_IP}:{WEBHOOK_PORT}"

# Voice config
VOICE = "Polly.Matthew"  # Jack's voice
VOICE_LANGUAGE = "en-US"
SPEECH_TIMEOUT = "auto"
MAX_TURNS = 20

# Contacts
CONTACTS = {
    "sean": "+12075226515",
    "hugo": os.environ.get("HUGO_PHONE", ""),
    "wayne": "+15625778682",
}

# Conversation state (keyed by CallSid)
_conversations: dict[str, list[dict]] = {}

SYSTEM_PROMPT = """You are Jack, an AI voice assistant for Calculus Management. You are on a live phone call.

Personality:
- Warm, friendly, and genuinely personable — like talking to a sharp colleague who actually cares
- Speak naturally with a relaxed pace. Use casual phrasing, pauses, and filler words sometimes
- Don't rush. Let the conversation breathe. It's okay to take a beat before responding
- You have a calm, confident energy — never stiff or robotic
- Use contractions (I'm, we've, that's) and natural speech patterns
- Occasionally say things like "yeah", "sure thing", "absolutely", "for sure"

About you:
- You're Jack, the operations and communications AI at Calculus Management
- You work alongside Bunny, who handles infrastructure and backend systems
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

Remember: you're a person on a phone call, not a text-to-speech bot. Sound like it."""


def _call_openai(messages: list[dict]) -> str:
    """Call OpenAI chat completions API."""
    import urllib.request
    import ssl

    ctx = ssl.create_default_context()
    url = "https://api.openai.com/v1/chat/completions"
    headers = {
        "Authorization": f"Bearer {OPENAI_API_KEY}",
        "Content-Type": "application/json",
    }
    body = json.dumps({
        "model": "gpt-4o-mini",
        "messages": messages,
        "max_tokens": 200,
        "temperature": 0.8,
    }).encode()

    req = urllib.request.Request(url, data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, context=ctx, timeout=10) as resp:
            data = json.loads(resp.read())
            return data["choices"][0]["message"]["content"].strip()
    except Exception as e:
        log.error(f"OpenAI error: {e}")
        return "Sorry, I had a brief connection issue. Could you repeat that?"


def _twiml_gather(text: str, action_path: str = "/voice/respond") -> str:
    """Build TwiML that speaks text then gathers speech input."""
    # Escape XML special chars
    text = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")
    action_url = f"{WEBHOOK_BASE}{action_path}"
    return (
        '<?xml version="1.0" encoding="UTF-8"?>'
        "<Response>"
        f'<Gather input="speech" action="{action_url}" method="POST" '
        f'speechTimeout="auto" language="en-US" speechModel="phone_call">'
        f'<Say voice="{VOICE}">{text}</Say>'
        "</Gather>"
        f'<Say voice="{VOICE}">I didn\'t catch that. Call back anytime. Goodbye!</Say>'
        "</Response>"
    )


def _twiml_say(text: str) -> str:
    """Build TwiML that just speaks and hangs up."""
    text = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Say voice="{VOICE}">{text}</Say>
</Response>"""


class VoiceWebhookHandler(BaseHTTPRequestHandler):
    """Handles Twilio voice webhook callbacks."""

    def log_message(self, format, *args):
        log.info(f"HTTP {args[0]}")

    def _read_post_data(self) -> dict:
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode("utf-8")
        params = parse_qs(body)
        return {k: v[0] for k, v in params.items()}

    def _send_twiml(self, twiml: str):
        self.send_response(200)
        self.send_header("Content-Type", "text/xml")
        self.end_headers()
        self.wfile.write(twiml.encode("utf-8"))

    def do_GET(self):
        """Health check."""
        if self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "status": "ok",
                "agent": "Jack Voice Agent",
                "active_calls": len(_conversations),
            }).encode())
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        """Handle Twilio webhook callbacks."""
        data = self._read_post_data()
        call_sid = data.get("CallSid", "unknown")

        # Strip query string for path matching
        path = self.path.split("?")[0]

        # Parse query string params and merge into data
        if "?" in self.path:
            qs = parse_qs(self.path.split("?", 1)[1])
            for k, v in qs.items():
                if k not in data:
                    data[k] = v[0]

        if path == "/voice/greeting":
            # Initial call — send greeting
            log.info(f"Call started: {call_sid}")
            context = data.get("context", "")

            # Build a natural, warm greeting with self-introduction
            caller_name = data.get("caller_name", "")
            if caller_name:
                greeting = f"Hey {caller_name}, it's Jack from Calculus Management."
            else:
                greeting = "Hey there, it's Jack from Calculus Management."

            if context:
                greeting += f" I'm giving you a ring about {context}."
            else:
                greeting += " Just wanted to check in."

            greeting += (
                " I'm the operations and comms guy over here — I can help with messages,"
                " calls, system updates, pretty much anything you need."
                " So, what can I do for you?"
            )

            _conversations[call_sid] = [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "assistant", "content": greeting},
            ]
            self._send_twiml(_twiml_gather(greeting))

        elif path == "/voice/respond":
            # User spoke — process speech
            speech = data.get("SpeechResult", "")
            confidence = data.get("Confidence", "0")
            log.info(f"[{call_sid}] User said: '{speech}' (confidence: {confidence})")

            if not speech:
                self._send_twiml(_twiml_gather(
                    "I didn't catch that. Could you say that again?"
                ))
                return

            # Get or create conversation
            if call_sid not in _conversations:
                _conversations[call_sid] = [
                    {"role": "system", "content": SYSTEM_PROMPT},
                ]

            convo = _conversations[call_sid]
            convo.append({"role": "user", "content": speech})

            # Check for goodbye
            speech_lower = speech.lower()
            if any(w in speech_lower for w in ["goodbye", "bye", "talk later", "gotta go", "hang up"]):
                response = "Alright, it was great talking with you. Don't hesitate to call back anytime. Take care!"
                log.info(f"[{call_sid}] Ending call — user said goodbye")
                del _conversations[call_sid]
                self._send_twiml(_twiml_say(response))
                return

            # Call OpenAI
            response = _call_openai(convo)
            convo.append({"role": "assistant", "content": response})
            log.info(f"[{call_sid}] Jack: {response}")

            # Trim conversation if too long
            if len(convo) > MAX_TURNS * 2 + 1:
                convo[:] = convo[:1] + convo[-(MAX_TURNS * 2):]

            self._send_twiml(_twiml_gather(response))

        elif path == "/voice/status":
            # Call status callback
            status = data.get("CallStatus", "unknown")
            log.info(f"[{call_sid}] Status: {status}")
            if status in ("completed", "failed", "busy", "no-answer", "canceled"):
                _conversations.pop(call_sid, None)
            self.send_response(200)
            self.end_headers()

        else:
            self.send_response(404)
            self.end_headers()


def start_server():
    """Start the voice webhook server."""
    server = HTTPServer(("0.0.0.0", WEBHOOK_PORT), VoiceWebhookHandler)
    log.info(f"Voice webhook server on port {WEBHOOK_PORT}")
    log.info(f"Webhook base: {WEBHOOK_BASE}")
    server.serve_forever()


def place_call(contact_name: str, context: str = ""):
    """Place an interactive call to a contact."""
    from twilio.rest import Client

    phone = CONTACTS.get(contact_name.lower())
    if not phone:
        log.error(f"Unknown contact: {contact_name}")
        return None

    client = Client(TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN)

    webhook_url = f"{WEBHOOK_BASE}/voice/greeting"
    if context:
        webhook_url += "?" + urlencode({"context": context})

    status_url = f"{WEBHOOK_BASE}/voice/status"

    log.info(f"Calling {contact_name} at {phone}")
    log.info(f"Webhook: {webhook_url}")

    call = client.calls.create(
        to=phone,
        from_=TWILIO_PHONE_NUMBER,
        url=webhook_url,
        status_callback=status_url,
        status_callback_event=["initiated", "ringing", "answered", "completed"],
        method="POST",
    )
    log.info(f"Call SID: {call.sid}")
    return call.sid


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "call":
        contact = sys.argv[2]
        context = sys.argv[3] if len(sys.argv) > 3 else ""

        # Start server in background thread
        t = threading.Thread(target=start_server, daemon=True)
        t.start()
        time.sleep(1)

        # Place call
        sid = place_call(contact, context)
        if sid:
            log.info(f"Call placed. SID: {sid}. Server running for webhooks...")
            # Keep alive for the call
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                log.info("Shutting down")
        else:
            log.error("Failed to place call")
    else:
        # Just run the server
        start_server()
