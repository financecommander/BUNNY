#!/usr/bin/env python3
"""Calculus AI — Unified Messaging Module
Channels: Signal, Telegram, Twilio (SMS + Voice + WhatsApp + VoIP), SendGrid Email
Supports agent identity routing: BUNNY and JACK
"""
import os
import subprocess
from pathlib import Path
from dotenv import load_dotenv

env_path = Path(__file__).parent / ".env"
load_dotenv(env_path)

# Agent identities
AGENTS = {
    "bunny": {
        "tag": "[BUNNY]",
        "voice": "Polly.Joanna",               # Female — works on Twilio
    },
    "jack": {
        "tag": "[JACK]",
        "voice": "Polly.Matthew",              # Male — works on Twilio
    },
}
DEFAULT_AGENT = "bunny"


def _tag_message(message: str, agent: str) -> str:
    """Prefix message with agent identity tag."""
    info = AGENTS.get(agent, AGENTS[DEFAULT_AGENT])
    return f"{info['tag']} {message}"


# -- Signal --
def send_signal(message: str, recipient: str = None) -> bool:
    cli = os.getenv("SIGNAL_CLI_PATH", "signal-cli")
    sender = os.getenv("SIGNAL_SENDER_NUMBER")
    recipient = recipient or os.getenv("SIGNAL_RECIPIENT_NUMBER")
    if not sender or not recipient:
        print("[SIGNAL] Missing sender/recipient in .env")
        return False
    try:
        result = subprocess.run(
            [cli, "-u", sender, "send", "-m", message, recipient],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode == 0:
            print(f"[SIGNAL] Sent to {recipient}")
            return True
        else:
            print(f"[SIGNAL] Error: {result.stderr}")
            return False
    except Exception as e:
        print(f"[SIGNAL] Exception: {e}")
        return False


# -- Telegram --
def send_telegram(message: str, chat_id: str = None) -> bool:
    import requests
    token = os.getenv("TELEGRAM_BOT_TOKEN")
    chat_id = chat_id or os.getenv("TELEGRAM_CHAT_ID")
    if not token or not chat_id or token == "CHANGEME":
        print("[TELEGRAM] Missing bot_token/chat_id in .env")
        return False
    try:
        url = f"https://api.telegram.org/bot{token}/sendMessage"
        resp = requests.post(url, json={
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "Markdown"
        }, timeout=10)
        if resp.ok:
            print(f"[TELEGRAM] Sent to chat {chat_id}")
            return True
        else:
            print(f"[TELEGRAM] Error: {resp.text}")
            return False
    except Exception as e:
        print(f"[TELEGRAM] Exception: {e}")
        return False


# -- Twilio SMS --
def send_twilio_sms(message: str, to: str = None) -> bool:
    from twilio.rest import Client
    sid = os.getenv("TWILIO_ACCOUNT_SID")
    token = os.getenv("TWILIO_AUTH_TOKEN")
    from_num = os.getenv("TWILIO_PHONE_NUMBER")
    to = to or os.getenv("TWILIO_RECIPIENT_NUMBER")
    if not all([sid, token, from_num, to]) or sid == "CHANGEME":
        print("[TWILIO SMS] Missing credentials in .env")
        return False
    try:
        client = Client(sid, token)
        msg = client.messages.create(body=message, from_=from_num, to=to)
        print(f"[TWILIO SMS] Sent to {to} (SID: {msg.sid})")
        return True
    except Exception as e:
        print(f"[TWILIO SMS] Exception: {e}")
        return False


# -- Twilio Voice Call --
def send_twilio_call(message: str, to: str = None, voice: str = None, agent: str = DEFAULT_AGENT) -> bool:
    """Place a voice call via Twilio with Polly Neural TTS."""
    from twilio.rest import Client
    sid = os.getenv("TWILIO_ACCOUNT_SID")
    token = os.getenv("TWILIO_AUTH_TOKEN")
    from_num = os.getenv("TWILIO_PHONE_NUMBER")
    to = to or os.getenv("TWILIO_RECIPIENT_NUMBER")
    if not all([sid, token, from_num, to]) or sid == "CHANGEME":
        print("[TWILIO CALL] Missing credentials in .env")
        return False
    if voice is None:
        voice = AGENTS.get(agent, AGENTS[DEFAULT_AGENT])["voice"]
    try:
        client = Client(sid, token)
        twiml = '<Response><Say voice="{v}">{m}</Say></Response>'.format(v=voice, m=message)
        call = client.calls.create(twiml=twiml, from_=from_num, to=to)
        print(f"[TWILIO CALL] Calling {to} (SID: {call.sid}) [voice: {voice}]")
        return True
    except Exception as e:
        print(f"[TWILIO CALL] Exception: {e}")
        return False


# -- Twilio VoIP Call (international, SIP-based, cheaper than PSTN) --
def send_twilio_voip_call(message: str, to: str = None, voice: str = None, agent: str = DEFAULT_AGENT) -> bool:
    """Place a VoIP call via Twilio. Works international without standard phone charges.
    Uses Twilio SIP — much cheaper for international destinations like Hugo.
    Rates: ~$0.01-0.04/min vs $0.10-2.00/min for standard international.
    """
    from twilio.rest import Client
    sid = os.getenv("TWILIO_ACCOUNT_SID")
    token = os.getenv("TWILIO_AUTH_TOKEN")
    from_num = os.getenv("TWILIO_PHONE_NUMBER")
    to = to or os.getenv("TWILIO_RECIPIENT_NUMBER")
    if not all([sid, token, from_num, to]) or sid == "CHANGEME":
        print("[TWILIO VOIP] Missing credentials in .env")
        return False
    if voice is None:
        voice = AGENTS.get(agent, AGENTS[DEFAULT_AGENT])["voice"]
    try:
        client = Client(sid, token)
        twiml = '<Response><Say voice="{v}">{m}</Say></Response>'.format(v=voice, m=message)
        call = client.calls.create(twiml=twiml, from_=from_num, to=to)
        print(f"[TWILIO VOIP] Calling {to} (SID: {call.sid}) [voice: {voice}]")
        return True
    except Exception as e:
        print(f"[TWILIO VOIP] Exception: {e}")
        return False


# -- Twilio WhatsApp --
def send_whatsapp(message: str, to: str = None) -> bool:
    """Send a WhatsApp message via Twilio WhatsApp Business API.
    Requires WhatsApp-enabled Twilio number or sandbox.
    ~$0.005/msg outbound.
    """
    from twilio.rest import Client
    sid = os.getenv("TWILIO_ACCOUNT_SID")
    token = os.getenv("TWILIO_AUTH_TOKEN")
    wa_from = os.getenv("TWILIO_WHATSAPP_NUMBER", os.getenv("TWILIO_PHONE_NUMBER"))
    to = to or os.getenv("TWILIO_RECIPIENT_NUMBER")
    if not all([sid, token, wa_from, to]) or sid == "CHANGEME":
        print("[WHATSAPP] Missing credentials in .env")
        return False
    # WhatsApp numbers must be prefixed with 'whatsapp:'
    if not wa_from.startswith("whatsapp:"):
        wa_from = f"whatsapp:{wa_from}"
    if not to.startswith("whatsapp:"):
        to_wa = f"whatsapp:{to}"
    else:
        to_wa = to
    try:
        client = Client(sid, token)
        msg = client.messages.create(
            body=message,
            from_=wa_from,
            to=to_wa
        )
        print(f"[WHATSAPP] Sent to {to} (SID: {msg.sid})")
        return True
    except Exception as e:
        print(f"[WHATSAPP] Exception: {e}")
        return False


# -- SendGrid Email --
def send_sendgrid_email(to_email: str, subject: str, body: str,
                        from_email: str = None, agent: str = DEFAULT_AGENT) -> bool:
    """Send email via SendGrid API.
    Works alongside Gmail — SendGrid for transactional/agent-sent emails,
    Gmail for per-user inbox management.
    """
    from sendgrid import SendGridAPIClient
    from sendgrid.helpers.mail import Mail

    api_key = os.getenv("SENDGRID_API_KEY")
    from_email = from_email or os.getenv("SENDGRID_FROM_EMAIL", "jack@calculusmanagement.com")
    if not api_key or api_key == "CHANGEME":
        print("[SENDGRID] Missing SENDGRID_API_KEY in .env")
        return False
    try:
        agent_info = AGENTS.get(agent, AGENTS[DEFAULT_AGENT])
        agent_name = "Jack from Calculus" if agent == "jack" else "Bunny from Calculus"
        mail = Mail(
            from_email=(from_email, agent_name),
            to_emails=to_email,
            subject=subject,
            plain_text_content=body,
        )
        sg = SendGridAPIClient(api_key)
        response = sg.send(mail)
        print(f"[SENDGRID] Sent to {to_email} (status: {response.status_code})")
        return response.status_code in (200, 201, 202)
    except Exception as e:
        print(f"[SENDGRID] Exception: {e}")
        return False


# -- Unified Interface --
def send_message(message: str, channel: str = "all", agent: str = DEFAULT_AGENT, **kwargs) -> dict:
    """Send message via specified channel(s) with agent identity.

    Channels: signal, telegram, sms, call, voip, whatsapp, email, all
    Agents: bunny, jack

    Extra kwargs for email: to_email, subject (defaults provided)
    """
    tagged = _tag_message(message, agent)
    results = {}

    if channel in ("signal", "all"):
        results["signal"] = send_signal(tagged)
    if channel in ("telegram", "all"):
        results["telegram"] = send_telegram(tagged)
    if channel in ("sms", "twilio_sms", "all"):
        results["twilio_sms"] = send_twilio_sms(tagged)
    if channel in ("call", "twilio_call"):
        results["twilio_call"] = send_twilio_call(message, agent=agent)
    if channel in ("voip", "twilio_voip"):
        to = kwargs.get("to")
        results["twilio_voip"] = send_twilio_voip_call(message, to=to, agent=agent)
    if channel in ("whatsapp", "wa"):
        to = kwargs.get("to")
        results["whatsapp"] = send_whatsapp(tagged, to=to)
    if channel in ("email", "sendgrid"):
        to_email = kwargs.get("to_email", os.getenv("SENDGRID_DEFAULT_TO", ""))
        subject = kwargs.get("subject", f"Message from {agent.title()}")
        results["sendgrid"] = send_sendgrid_email(to_email, subject, message, agent=agent)

    return results


# -- Contact Profiles (per-user preferred channels) --
CONTACTS = {
    "sean": {
        "phone": "+12075226515",
        "channels": ["sms", "call", "telegram", "whatsapp", "email"],
    },
    "hugo": {
        "phone": os.getenv("HUGO_PHONE", ""),
        "telegram_chat_id": os.getenv("HUGO_TELEGRAM_CHAT_ID", ""),
        "email": os.getenv("HUGO_EMAIL", ""),
        "channels": ["telegram", "whatsapp", "signal", "voip"],  # international — no standard calls
    },
    "wayne": {
        "phone": "+15625778682",
        "email": "wayneorkin@icloud.com",
        "channels": ["sms", "call", "whatsapp", "email"],
    },
}


def send_to_contact(contact_name: str, message: str, agent: str = DEFAULT_AGENT,
                    channel: str = None) -> dict:
    """Send to a known contact using their preferred channel(s).
    If no channel specified, uses first available from their profile.
    Hugo = international, routes via telegram/whatsapp/signal/voip (no PSTN calls).
    """
    contact = CONTACTS.get(contact_name.lower())
    if not contact:
        print(f"[COMMS] Unknown contact: {contact_name}")
        return {"error": f"Unknown contact: {contact_name}"}

    if channel:
        channels = [channel]
    else:
        channels = contact.get("channels", ["telegram"])

    results = {}
    for ch in channels:
        if ch == "telegram" and contact.get("telegram_chat_id"):
            tagged = _tag_message(message, agent)
            results["telegram"] = send_telegram(tagged, chat_id=contact["telegram_chat_id"])
        elif ch == "whatsapp" and contact.get("phone"):
            tagged = _tag_message(message, agent)
            results["whatsapp"] = send_whatsapp(tagged, to=contact["phone"])
        elif ch == "signal" and contact.get("phone"):
            tagged = _tag_message(message, agent)
            results["signal"] = send_signal(tagged, recipient=contact["phone"])
        elif ch == "voip" and contact.get("phone"):
            results["voip"] = send_twilio_voip_call(message, to=contact["phone"], agent=agent)
        elif ch == "sms" and contact.get("phone"):
            tagged = _tag_message(message, agent)
            results["sms"] = send_twilio_sms(tagged, to=contact["phone"])
        elif ch == "call" and contact.get("phone"):
            results["call"] = send_twilio_call(message, to=contact["phone"], agent=agent)
        elif ch == "email" and contact.get("email"):
            results["email"] = send_sendgrid_email(
                contact["email"], f"Message from {agent.title()}", message, agent=agent
            )
        if results.get(ch):
            break  # stop after first successful channel
    return results


if __name__ == "__main__":
    import sys
    if len(sys.argv) < 2:
        print("Usage:")
        print("  send.py <message> [channel] [agent]")
        print("  send.py --contact <name> <message> [channel] [agent]")
        print("")
        print("Channels: signal, telegram, sms, call, voip, whatsapp, email, all")
        print("Agents: bunny, jack")
        print("Contacts: sean, hugo")
        sys.exit(0)

    if sys.argv[1] == "--contact":
        name = sys.argv[2]
        msg = sys.argv[3] if len(sys.argv) > 3 else "Calculus AI system operational."
        ch = sys.argv[4] if len(sys.argv) > 4 else None
        ag = sys.argv[5] if len(sys.argv) > 5 else DEFAULT_AGENT
        print(f"Sending to contact: {name} | Channel: {ch or 'auto'} | Agent: {ag}")
        print(send_to_contact(name, msg, ag, ch))
    else:
        msg = sys.argv[1]
        ch = sys.argv[2] if len(sys.argv) > 2 else "all"
        ag = sys.argv[3] if len(sys.argv) > 3 else DEFAULT_AGENT
        print(f"Sending via: {ch} | Agent: {ag}")
        print(send_message(msg, ch, ag))
