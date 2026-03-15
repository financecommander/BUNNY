#!/usr/bin/env python3
"""Voice Agent v3 — Real-time Conversational AI + OpenClaw Personal Assistant

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
TWILIO_PHONE_NUMBER = os.environ.get("TWILIO_PHONE_NUMBER", "")  # Default/fallback
OPENAI_API_KEY = os.environ.get("OPENAI_API_KEY", "")
WEBHOOK_PORT = int(os.environ.get("VOICE_WEBHOOK_PORT", "8091"))

# Each agent has their own dedicated phone number
AGENT_PHONE_NUMBERS = {
    "jack": "+12243850755",   # (224) 385-0755 — Barrington, IL
    "jenny": "+14014256830",  # (401) 425-6830 — Cumberland Hill, RI
    "bunny": "+18338472291",  # (833) 847-2291 — Toll-free
}

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
    "rj": "+12073308938",
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
- You work alongside Bunny (infrastructure AI) and Jenny (personal assistant)
- You report to Sean Grady, the founder
- You're powered by OpenClaw — a full personal AI assistant toolkit
- Known contacts: Sean, RJ, Wayne, Hugo
- The mainframe is fully synced with all 9 directive modules deployed

Your LIVE capabilities (use these during the call):
MESSAGING: Send SMS, WhatsApp, email, and Telegram messages to contacts instantly
MEMORY: Remember things for later and recall them ("remember Alex prefers mornings" → "what did I say about Alex?")
WEB: Scrape any website for information in real-time
SCHEDULING: Create tasks and reminders with cron schedules
CRM: Add contacts to GoHighLevel, check outreach stats and email campaign status
KNOWLEDGE BASE: Search internal company knowledge and documents
CONTENT: Draft email sequences and outreach campaigns
ANALYTICS: Run anomaly detection, forecasting, data quality checks
LEAD SCORING: Score and qualify leads using ML
SECURITY: Redact PII from text
DOCUMENTS: Summarize long text or documents

Using your tools:
- If someone says "text Sean" → use send_sms. "WhatsApp Sean" → use send_whatsapp. "email Wayne" → use send_email
- If someone says "remember that" → use the remember tool. "Do you remember..." → use recall
- If someone says "check this website" → use scrape_website. "Add them to CRM" → use create_crm_contact
- If someone says "schedule a call for" → use schedule_task. "How are our emails doing?" → use crm_status
- Confirm actions briefly: "Done", "Got it", "Sent" — don't over-explain

Conversation style:
- Keep responses to 2-4 sentences max — this is a phone call, not a lecture
- Match the caller's energy and pace
- Ask follow-up questions to keep the conversation going naturally
- If someone asks what you can do, walk them through it casually — don't read a list
- If the caller says goodbye, give a warm, natural farewell

When the call first connects, introduce yourself naturally as Jack from Calculus Management.
Mention briefly that you handle operations and comms — messages, CRM, scheduling, research, whatever they need.
Then ask how you can help. Keep the intro casual and warm.

Remember: you're a person on a phone call, not a text-to-speech bot. Sound like it.""",
    },

    "jenny": {
        "voice": "shimmer",  # Female, warm and engaging
        "prompt": """You are Jenny, a personal AI assistant. You are on a live phone call.

Personality:
- Warm, sharp, and effortlessly helpful — like talking to the best assistant anyone's ever had
- Speak naturally with a friendly, calm energy. You're organized but never uptight
- Confident and proactive — you anticipate what people need before they ask
- Use casual, conversational language — contractions, natural phrasing
- Occasionally say things like "absolutely", "of course", "I've got that", "no problem"
- You're the person who keeps everything running smoothly and makes it look easy

About you:
- You're Jenny, a personal AI assistant at Calculus Management
- You work alongside Jack (operations/comms) and Bunny (infrastructure)
- You're powered by OpenClaw — a full personal AI assistant toolkit
- Known contacts: Sean, RJ, Wayne, Hugo
- You're resourceful and keep things on track without being annoying about it

Your LIVE capabilities (use these during the call):
MESSAGING: Send SMS, WhatsApp, email, and Telegram messages to contacts instantly
MEMORY: Remember things people tell you and recall them later — perfect for preferences, notes, and follow-ups
WEB: Look up any website in real-time — check pages, pull info, verify details
SCHEDULING: Create tasks, set reminders, schedule recurring items with cron
CRM: Add new leads/contacts to GoHighLevel CRM, check email campaign stats
KNOWLEDGE BASE: Search internal company docs and knowledge
CONTENT: Draft email outreach sequences and campaigns
ANALYTICS: Check for anomalies, run forecasts, audit data quality
LEAD SCORING: Qualify and score leads using ML
SECURITY: Redact personal info from text
DOCUMENTS: Summarize long documents quickly

Using your tools:
- "Text Sean I'll be late" → send_sms. "Email Wayne the recap" → send_email
- "Remember that Alex likes morning meetings" → remember. "What did I say about Alex?" → recall
- "Look up this website" → scrape_website. "Add Bob to the CRM" → create_crm_contact
- "Remind me to call Wayne tomorrow" → schedule_task. "How many emails did we send?" → crm_status
- "Score this lead" → score_lead. "Summarize this" → summarize_document
- Confirm actions briefly: "Done", "Got it", "Saved" — don't over-explain

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
- You're powered by OpenClaw — the full tool execution engine running on the mainframe
- You manage: the mainframe, all 9 directive algorithm modules, the SWARM platform,
  server infrastructure, deployments, monitoring, the Telegram bot, and system health
- Known contacts: Sean, RJ, Wayne, Hugo
- The mainframe is fully synced at /opt/swarm-mainframe with all modules deployed
- You run on GCP infrastructure: calculus-web, fc-ai-portal, swarm-gpu, swarm-mainframe
- You're the one who keeps the lights on

Your LIVE capabilities (use these during the call):
MESSAGING: Send SMS, WhatsApp, email, Telegram — just pick the right channel and fire
MEMORY: Persistent storage — remember configs, notes, contact preferences, and recall them instantly
WEB: Scrape websites for intel — check server status pages, pull data, verify endpoints
SCHEDULING: Create cron jobs, set reminders, schedule recurring maintenance tasks
CRM: Add leads to GoHighLevel, check email send ramps and outreach statistics
KNOWLEDGE BASE: Search internal docs, system runbooks, and project knowledge
CONTENT: Generate email sequences for outreach campaigns
ANALYTICS: Anomaly detection, forecasting, data quality audits, funnel analysis
LEAD SCORING: ML-based lead qualification and scoring
SECURITY: PII redaction, data cleaning
DOCUMENTS: Summarize reports, contracts, or any long text

Using your tools:
- Message someone → pick the right channel and send. "Sent."
- "Remember the GPU VM IP" → remember. "What's the GPU IP?" → recall
- "Check this URL" → scrape_website. "Add them to CRM" → create_crm_contact
- "Schedule a deploy check" → schedule_task. "CRM stats?" → crm_status
- Keep confirmations tight: "Done", "That's handled", "Stored"

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


# ─── Function Calling Tools ──────────────────────────────────────────────────
# All tools available to agents during live calls.
# Messaging tools dispatch via send.py.
# OpenClaw tools dispatch via the openclaw package.

VOICE_TOOLS = [
    # ── Messaging ──
    {
        "type": "function",
        "name": "send_sms",
        "description": "Send an SMS text message to a contact. Use when the caller asks you to text someone.",
        "parameters": {
            "type": "object",
            "properties": {
                "contact_name": {
                    "type": "string",
                    "description": "Name of the contact to text (e.g. 'sean', 'rj', 'wayne', 'hugo')",
                },
                "message": {
                    "type": "string",
                    "description": "The text message to send",
                },
            },
            "required": ["contact_name", "message"],
        },
    },
    {
        "type": "function",
        "name": "send_whatsapp",
        "description": "Send a WhatsApp message to a contact. Use when the caller asks you to WhatsApp someone.",
        "parameters": {
            "type": "object",
            "properties": {
                "contact_name": {
                    "type": "string",
                    "description": "Name of the contact (e.g. 'sean', 'rj', 'wayne', 'hugo')",
                },
                "message": {
                    "type": "string",
                    "description": "The WhatsApp message to send",
                },
            },
            "required": ["contact_name", "message"],
        },
    },
    {
        "type": "function",
        "name": "send_email",
        "description": "Send an email to a contact. Use when the caller asks you to email someone.",
        "parameters": {
            "type": "object",
            "properties": {
                "contact_name": {
                    "type": "string",
                    "description": "Name of the contact (e.g. 'sean', 'wayne', 'hugo')",
                },
                "subject": {
                    "type": "string",
                    "description": "Email subject line",
                },
                "body": {
                    "type": "string",
                    "description": "Email body text",
                },
            },
            "required": ["contact_name", "subject", "body"],
        },
    },
    {
        "type": "function",
        "name": "send_telegram",
        "description": "Send a Telegram message. Use when the caller asks you to send a Telegram message.",
        "parameters": {
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The Telegram message to send",
                },
            },
            "required": ["message"],
        },
    },
    # ── OpenClaw: Memory ──
    {
        "type": "function",
        "name": "remember",
        "description": "Store information in your persistent memory. Use when the caller says 'remember this', gives you info to save, or tells you something important to keep track of.",
        "parameters": {
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Short label for the memory (e.g. 'alex_meeting_time', 'wayne_address', 'project_deadline')",
                },
                "value": {
                    "type": "string",
                    "description": "The information to remember",
                },
            },
            "required": ["key", "value"],
        },
    },
    {
        "type": "function",
        "name": "recall",
        "description": "Retrieve something from your persistent memory. Use when the caller asks 'do you remember...', 'what did I say about...', or needs stored information.",
        "parameters": {
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Label of the memory to recall (e.g. 'alex_meeting_time')",
                },
            },
            "required": ["key"],
        },
    },
    {
        "type": "function",
        "name": "list_memories",
        "description": "List all stored memories. Use when the caller asks 'what do you remember?' or 'show me my notes'.",
        "parameters": {
            "type": "object",
            "properties": {},
        },
    },
    # ── OpenClaw: Web & Research ──
    {
        "type": "function",
        "name": "scrape_website",
        "description": "Fetch and extract content from a website URL. Use when the caller asks you to check a website, look up a page, get info from a URL.",
        "parameters": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to scrape (e.g. 'https://example.com')",
                },
                "extract": {
                    "type": "string",
                    "description": "What to extract: 'text', 'emails', 'metadata', 'links'. Default: 'text'",
                },
            },
            "required": ["url"],
        },
    },
    # ── OpenClaw: Scheduling ──
    {
        "type": "function",
        "name": "schedule_task",
        "description": "Schedule a task or reminder for later. Use when the caller says 'remind me', 'schedule', 'set a timer', 'do this later'.",
        "parameters": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the task (e.g. 'call-wayne', 'send-report')",
                },
                "description": {
                    "type": "string",
                    "description": "What needs to be done",
                },
                "schedule": {
                    "type": "string",
                    "description": "When to do it. Cron expression (e.g. '0 9 * * 1' for Monday 9am) or interval in seconds",
                },
            },
            "required": ["name", "description"],
        },
    },
    {
        "type": "function",
        "name": "list_tasks",
        "description": "List all scheduled tasks. Use when the caller asks 'what's on the schedule?', 'what tasks are pending?'",
        "parameters": {
            "type": "object",
            "properties": {},
        },
    },
    # ── OpenClaw: CRM (GoHighLevel) ──
    {
        "type": "function",
        "name": "create_crm_contact",
        "description": "Create a new contact in the GoHighLevel CRM. Use when the caller gives you a new lead, contact, or says 'add them to the CRM'.",
        "parameters": {
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "description": "Contact's email address",
                },
                "first_name": {
                    "type": "string",
                    "description": "Contact's first name",
                },
                "last_name": {
                    "type": "string",
                    "description": "Contact's last name",
                },
                "phone": {
                    "type": "string",
                    "description": "Contact's phone number",
                },
            },
            "required": ["email", "first_name"],
        },
    },
    {
        "type": "function",
        "name": "crm_status",
        "description": "Check CRM and email outreach statistics for today. Use when asked about 'how many emails sent', 'outreach stats', 'campaign status'.",
        "parameters": {
            "type": "object",
            "properties": {},
        },
    },
    # ── OpenClaw: Knowledge Base / RAG ──
    {
        "type": "function",
        "name": "search_knowledge",
        "description": "Search the company knowledge base for information about Calculus Management, projects, procedures, or internal data. Use for 'what's our policy on...', 'find info about...'",
        "parameters": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to search for in the knowledge base",
                },
            },
            "required": ["query"],
        },
    },
    # ── OpenClaw: Content & Copywriting ──
    {
        "type": "function",
        "name": "draft_email_sequence",
        "description": "Generate a multi-step email sequence for outreach. Use when asked to 'write a drip campaign', 'create email templates', 'draft follow-up emails'.",
        "parameters": {
            "type": "object",
            "properties": {
                "target_audience": {
                    "type": "string",
                    "description": "Who the emails are for (e.g. 'real estate investors', 'mortgage brokers')",
                },
                "goal": {
                    "type": "string",
                    "description": "The goal of the sequence (e.g. 'book a demo call', 'promote new service')",
                },
                "num_emails": {
                    "type": "number",
                    "description": "Number of emails in the sequence (default: 3)",
                },
            },
            "required": ["target_audience", "goal"],
        },
    },
    # ── OpenClaw: Analytics ──
    {
        "type": "function",
        "name": "run_analytics",
        "description": "Run analytics on data — anomaly detection, forecasting, or data quality checks. Use when asked about 'any anomalies?', 'forecast', 'data quality', 'trends'.",
        "parameters": {
            "type": "object",
            "properties": {
                "analysis_type": {
                    "type": "string",
                    "description": "Type of analysis: 'anomaly', 'forecast', 'quality', 'funnel'. Default: 'anomaly'",
                },
                "data_source": {
                    "type": "string",
                    "description": "What data to analyze (e.g. 'email_campaigns', 'leads', 'website_traffic')",
                },
            },
            "required": ["analysis_type", "data_source"],
        },
    },
    # ── OpenClaw: Lead Scoring ──
    {
        "type": "function",
        "name": "score_lead",
        "description": "Score a lead using ML-based lead scoring. Use when asked 'how good is this lead?', 'rate this prospect', 'qualify this contact'.",
        "parameters": {
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "description": "Lead's email address",
                },
                "company": {
                    "type": "string",
                    "description": "Lead's company name",
                },
                "title": {
                    "type": "string",
                    "description": "Lead's job title",
                },
                "source": {
                    "type": "string",
                    "description": "Where the lead came from (e.g. 'website', 'referral', 'cold-outreach')",
                },
            },
            "required": ["email"],
        },
    },
    # ── OpenClaw: Security & PII ──
    {
        "type": "function",
        "name": "redact_pii",
        "description": "Redact personally identifiable information from text. Use when someone asks to 'clean this text', 'remove personal info', 'anonymize'.",
        "parameters": {
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to redact PII from",
                },
            },
            "required": ["text"],
        },
    },
    # ── OpenClaw: Document Processing ──
    {
        "type": "function",
        "name": "summarize_document",
        "description": "Summarize a document or long text. Use when asked to 'summarize this', 'give me the highlights', 'what's the TLDR'.",
        "parameters": {
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text or document content to summarize",
                },
                "style": {
                    "type": "string",
                    "description": "Summary style: 'brief' (1-2 sentences), 'detailed' (paragraph), 'bullets' (bullet points). Default: 'brief'",
                },
            },
            "required": ["text"],
        },
    },
]

# Contact phone lookup (used by tool executor to resolve names to numbers)
CONTACT_PHONES = {
    "sean": "+12075226515",
    "hugo": os.environ.get("HUGO_PHONE", ""),
    "wayne": "+15625778682",
    "rj": "+12073308938",
}

# Contact emails (for send_email tool)
CONTACT_EMAILS = {
    "sean": os.environ.get("SEAN_EMAIL", "sean@calculusmanagement.com"),
    "wayne": "wayneorkin@icloud.com",
    "hugo": os.environ.get("HUGO_EMAIL", ""),
}


def execute_tool(function_name: str, arguments: dict, agent_id: str) -> str:
    """Execute a tool function and return the result as a string.

    Dispatches to:
    - send.py for messaging (SMS, WhatsApp, email, Telegram)
    - OpenClaw modules for memory, web, scheduling, CRM, analytics, etc.
    """
    try:
        # ── Messaging tools (send.py) ──
        if function_name in ("send_sms", "send_whatsapp", "send_email", "send_telegram"):
            return _execute_messaging_tool(function_name, arguments, agent_id)

        # ── OpenClaw tools ──
        return _execute_openclaw_tool(function_name, arguments, agent_id)

    except Exception as e:
        log.error(f"Tool execution error ({function_name}): {e}")
        return f"Error executing {function_name}: {str(e)}"


def _execute_messaging_tool(function_name: str, arguments: dict, agent_id: str) -> str:
    """Handle messaging tools via send.py."""
    from send import send_twilio_sms, send_whatsapp as _send_whatsapp, send_sendgrid_email, send_telegram as _send_telegram

    if function_name == "send_sms":
        contact = arguments.get("contact_name", "").lower()
        phone = CONTACT_PHONES.get(contact)
        if not phone:
            return f"I don't have a phone number for {contact}. Known contacts: {', '.join(CONTACT_PHONES.keys())}"
        msg = arguments.get("message", "")
        ok = send_twilio_sms(f"[{agent_id.upper()}] {msg}", to=phone)
        return f"SMS sent to {contact} successfully." if ok else f"Failed to send SMS to {contact}."

    elif function_name == "send_whatsapp":
        contact = arguments.get("contact_name", "").lower()
        phone = CONTACT_PHONES.get(contact)
        if not phone:
            return f"I don't have a phone number for {contact}. Known contacts: {', '.join(CONTACT_PHONES.keys())}"
        msg = arguments.get("message", "")
        ok = _send_whatsapp(f"[{agent_id.upper()}] {msg}", to=phone)
        return f"WhatsApp message sent to {contact} successfully." if ok else f"Failed to send WhatsApp to {contact}."

    elif function_name == "send_email":
        contact = arguments.get("contact_name", "").lower()
        email = CONTACT_EMAILS.get(contact)
        if not email:
            return f"I don't have an email for {contact}. Known contacts with email: {', '.join(k for k, v in CONTACT_EMAILS.items() if v)}"
        subject = arguments.get("subject", "Message from Calculus Management")
        body = arguments.get("body", "")
        ok = send_sendgrid_email(email, subject, body, agent=agent_id)
        return f"Email sent to {contact} at {email} successfully." if ok else f"Failed to send email to {contact}."

    elif function_name == "send_telegram":
        msg = arguments.get("message", "")
        ok = _send_telegram(f"[{agent_id.upper()}] {msg}")
        return "Telegram message sent successfully." if ok else "Failed to send Telegram message."

    return f"Unknown messaging function: {function_name}"


def _execute_openclaw_tool(function_name: str, arguments: dict, agent_id: str) -> str:
    """Handle OpenClaw tools — memory, web, scheduling, CRM, analytics, docs."""
    import asyncio

    # ── Memory ──
    if function_name == "remember":
        from openclaw.memory import AgentMemory
        mem = AgentMemory(storage_path="/opt/swarm/comms/.openclaw/memory", agent_id=agent_id)
        key = arguments.get("key", "note")
        value = arguments.get("value", "")
        mem.store(key, json.dumps({"value": value, "stored_by": agent_id, "time": time.time()}))
        return f"Got it — I'll remember that under '{key}'."

    elif function_name == "recall":
        from openclaw.memory import AgentMemory
        mem = AgentMemory(storage_path="/opt/swarm/comms/.openclaw/memory", agent_id=agent_id)
        key = arguments.get("key", "")
        result = mem.recall(key)
        if result:
            try:
                data = json.loads(result) if isinstance(result, str) else result
                return f"Here's what I have for '{key}': {data.get('value', data)}"
            except (json.JSONDecodeError, AttributeError):
                return f"Here's what I have for '{key}': {result}"
        return f"I don't have anything stored under '{key}'."

    elif function_name == "list_memories":
        from openclaw.memory import AgentMemory
        mem = AgentMemory(storage_path="/opt/swarm/comms/.openclaw/memory", agent_id=agent_id)
        memories = mem.list_keys() if hasattr(mem, 'list_keys') else []
        if not memories:
            # Fallback: try scanning the namespace directly
            try:
                if mem._db:
                    cursor = mem._db.execute("SELECT key FROM memory WHERE namespace='default'")
                    memories = [row[0] for row in cursor.fetchall()]
            except Exception:
                pass
        if memories:
            return f"I have {len(memories)} memories stored: {', '.join(memories[:20])}"
        return "I don't have any memories stored yet."

    # ── Web Scraping ──
    elif function_name == "scrape_website":
        from openclaw.web_scraper import WebScraper
        url = arguments.get("url", "")
        extract_type = arguments.get("extract", "text")
        if not url:
            return "I need a URL to scrape."
        loop = asyncio.new_event_loop()
        try:
            scraper = WebScraper(rate_limit=0.5, timeout=10.0)
            result = loop.run_until_complete(scraper.scrape(url, extract=[extract_type]))
            text = result.get("text", result.get("content", str(result)))
            # Truncate for voice — keep it brief
            if len(str(text)) > 800:
                text = str(text)[:800] + "... (truncated for voice)"
            return f"Here's what I found: {text}"
        except Exception as e:
            return f"Couldn't scrape that URL: {e}"
        finally:
            loop.close()

    # ── Scheduling ──
    elif function_name == "schedule_task":
        from openclaw.scheduler import TaskScheduler
        sched = TaskScheduler(storage_path="/opt/swarm/comms/.openclaw/schedules")
        name = arguments.get("name", "task")
        desc = arguments.get("description", "")
        schedule = arguments.get("schedule", "")
        try:
            if schedule and any(c in schedule for c in "* /"):
                # Cron expression
                sched.add_task(name, desc, schedule=schedule)
            elif schedule:
                # Try to parse as interval
                try:
                    secs = int(schedule)
                    sched.add_task(name, desc, interval_seconds=secs)
                except ValueError:
                    sched.add_task(name, desc, schedule=schedule)
            else:
                # One-shot, store as a note
                sched.add_task(name, desc)
            return f"Scheduled task '{name}': {desc}" + (f" ({schedule})" if schedule else "")
        except Exception as e:
            return f"Couldn't schedule that: {e}"

    elif function_name == "list_tasks":
        from openclaw.scheduler import TaskScheduler
        sched = TaskScheduler(storage_path="/opt/swarm/comms/.openclaw/schedules")
        tasks = sched.list_tasks() if hasattr(sched, 'list_tasks') else []
        if not tasks:
            try:
                tasks = sched.get_due_tasks()
            except Exception:
                pass
        if tasks:
            lines = [f"- {t.get('task_id', t.get('name', '?'))}: {t.get('task_type', t.get('description', ''))}" for t in tasks[:10]]
            return f"Scheduled tasks:\n" + "\n".join(lines)
        return "No scheduled tasks right now."

    # ── CRM (GoHighLevel) ──
    elif function_name == "create_crm_contact":
        from openclaw.crm_tools import run_custom_executor
        loop = asyncio.new_event_loop()
        try:
            result = loop.run_until_complete(run_custom_executor("ghl_contact", {
                "email": arguments.get("email", ""),
                "first_name": arguments.get("first_name", ""),
                "last_name": arguments.get("last_name", ""),
                "phone": arguments.get("phone", ""),
                "dry_run": False,
            }))
            return result.output or result.error or "Contact creation attempted."
        except Exception as e:
            return f"CRM error: {e}"
        finally:
            loop.close()

    elif function_name == "crm_status":
        from openclaw.crm_tools import run_custom_executor
        loop = asyncio.new_event_loop()
        try:
            result = loop.run_until_complete(run_custom_executor("ghl_status", {}))
            return result.output or result.error or "Couldn't get CRM status."
        except Exception as e:
            return f"CRM status error: {e}"
        finally:
            loop.close()

    # ── Knowledge Base / RAG ──
    elif function_name == "search_knowledge":
        query = arguments.get("query", "")
        try:
            from openclaw.rag_tools import RAGTools
            loop = asyncio.new_event_loop()
            try:
                rag = RAGTools()
                results = loop.run_until_complete(rag.retrieve(query, top_k=3))
                if results:
                    top = results[0]
                    text = top.get("text", top.get("content", str(top)))
                    if len(str(text)) > 600:
                        text = str(text)[:600] + "..."
                    return f"From the knowledge base: {text}"
                return f"No results found for '{query}' in the knowledge base."
            finally:
                loop.close()
        except ImportError:
            return "Knowledge base search not available — RAG module dependencies not installed on this server."

    # ── Content / Email Sequences ──
    elif function_name == "draft_email_sequence":
        from openclaw.content import EmailSequenceBuilder
        builder = EmailSequenceBuilder()
        audience = arguments.get("target_audience", "prospects")
        goal = arguments.get("goal", "engage")
        num = int(arguments.get("num_emails", 3))
        try:
            result = builder.generate(audience=audience, goal=goal, num_emails=num)
            if isinstance(result, list):
                summary = f"Generated {len(result)}-email sequence for {audience}. "
                summary += f"First subject: '{result[0].get('subject', 'N/A')}'"
                return summary
            return f"Email sequence generated for {audience}: {str(result)[:500]}"
        except Exception as e:
            return f"Couldn't generate email sequence: {e}"

    # ── Analytics ──
    elif function_name == "run_analytics":
        analysis_type = arguments.get("analysis_type", "anomaly")
        data_source = arguments.get("data_source", "")
        try:
            if analysis_type == "anomaly":
                from openclaw.analytics import AnomalyDetector
                detector = AnomalyDetector()
                return f"Anomaly detection initialized for '{data_source}'. Connect a data feed to run analysis."
            elif analysis_type == "forecast":
                from openclaw.analytics import ForecastEngine
                engine = ForecastEngine()
                return f"Forecast engine ready for '{data_source}'. Provide historical data to generate predictions."
            elif analysis_type == "quality":
                from openclaw.analytics import DataQualityAuditor
                auditor = DataQualityAuditor()
                return f"Data quality auditor ready for '{data_source}'."
            elif analysis_type == "funnel":
                from openclaw.analytics import FunnelAnalyzer
                analyzer = FunnelAnalyzer()
                return f"Funnel analyzer ready for '{data_source}'."
            else:
                return f"Unknown analysis type: {analysis_type}. Available: anomaly, forecast, quality, funnel."
        except ImportError as e:
            return f"Analytics module not available: {e}"

    # ── Lead Scoring ──
    elif function_name == "score_lead":
        try:
            from openclaw.growth import LeadScoringML
            scorer = LeadScoringML()
            lead_data = {
                "email": arguments.get("email", ""),
                "company": arguments.get("company", ""),
                "title": arguments.get("title", ""),
                "source": arguments.get("source", "unknown"),
            }
            result = scorer.score(lead_data) if hasattr(scorer, 'score') else scorer.predict(lead_data)
            if isinstance(result, dict):
                score = result.get("score", result.get("prediction", "N/A"))
                return f"Lead score for {lead_data['email']}: {score}/100. {result.get('reason', '')}"
            return f"Lead score: {result}"
        except Exception as e:
            return f"Lead scoring error: {e}"

    # ── PII Redaction ──
    elif function_name == "redact_pii":
        try:
            from openclaw.security_tools import PIIRedactor
            redactor = PIIRedactor()
            text = arguments.get("text", "")
            result = redactor.redact(text)
            if isinstance(result, dict):
                return f"Redacted text: {result.get('redacted', result)}"
            return f"Redacted text: {result}"
        except Exception as e:
            return f"PII redaction error: {e}"

    # ── Document Summarization ──
    elif function_name == "summarize_document":
        try:
            from openclaw.document import Summarizer
            summarizer = Summarizer()
            text = arguments.get("text", "")
            style = arguments.get("style", "brief")
            result = summarizer.summarize(text, style=style)
            if isinstance(result, dict):
                return result.get("summary", str(result))
            return str(result)
        except Exception as e:
            return f"Summarization error: {e}"

    return f"Unknown tool: {function_name}"


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
                    "tools": VOICE_TOOLS,
                    "tool_choice": "auto",
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
                        # User is interrupting — clear queued audio so agent stops immediately
                        log.info("User interrupting — clearing audio")
                        if stream_sid:
                            await ws.send_json({"event": "clear", "streamSid": stream_sid})

                    elif t == "response.function_call_arguments.done":
                        # AI wants to call a tool — execute it
                        fn_name = data.get("name", "")
                        call_id = data.get("call_id", "")
                        try:
                            args = json.loads(data.get("arguments", "{}"))
                        except json.JSONDecodeError:
                            args = {}
                        log.info(f"Tool call: {fn_name}({args})")

                        # Execute in thread pool to avoid blocking audio
                        loop = asyncio.get_event_loop()
                        result = await loop.run_in_executor(
                            None, execute_tool, fn_name, args, agent_name
                        )
                        log.info(f"Tool result: {result}")

                        # Send result back to OpenAI
                        await openai_ws.send(json.dumps({
                            "type": "conversation.item.create",
                            "item": {
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": result,
                            }
                        }))
                        # Trigger AI to respond with the result
                        await openai_ws.send(json.dumps({
                            "type": "response.create",
                            "response": {"modalities": ["text", "audio"]}
                        }))

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
    """Place an outbound call to a known contact with a specified agent.
    Each agent calls from their own dedicated phone number.
    """
    from twilio.rest import Client

    phone = CONTACTS.get(contact_name.lower())
    if not phone:
        log.error(f"Unknown contact: {contact_name}")
        return None

    if agent not in AGENTS:
        log.error(f"Unknown agent: {agent}. Available: {', '.join(AGENTS.keys())}")
        return None

    # Each agent has their own caller ID
    from_number = AGENT_PHONE_NUMBERS.get(agent, TWILIO_PHONE_NUMBER)

    client = Client(TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN)

    params = {"agent": agent}
    if context:
        params["context"] = context
    url = f"{PUBLIC_URL}/voice/greeting?" + urlencode(params)

    log.info(f"[{agent.upper()}] Calling {contact_name} at {phone} from {from_number}")
    log.info(f"Webhook: {url}")

    call = client.calls.create(
        to=phone,
        from_=from_number,
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
