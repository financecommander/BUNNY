#!/usr/bin/env python3
"""Calculus AI — Telegram Bot — Admin: Sean Grady (ID: 695112527)
Features:
  - System status, GPU, logs
  - SMS/Call/WhatsApp/Email dispatch
  - Active email scanning with urgent alert detection
  - Dictate emails or send comms via natural language
  - Collaboration routing for urgent/difficult tasks
"""
import telebot
import logging
import subprocess
import os
import threading
import time
import json
from datetime import datetime
from pathlib import Path

TOKEN = '8781157185:AAFF-51hRffKC-mYN3JXEbQDrgCEMf4-8R0'
ADMIN_ID = 695112527
bot = telebot.TeleBot(TOKEN)

LOG_DIR = '/opt/swarm/comms/logs'
os.makedirs(LOG_DIR, exist_ok=True)

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - CALCULUS_BOT - %(levelname)s - %(message)s',
    handlers=[
        logging.FileHandler(f'{LOG_DIR}/bot.log'),
        logging.StreamHandler()
    ]
)
logger = logging.getLogger(__name__)

# Email scan state
email_scan_active = False
email_scan_interval = 300  # 5 minutes default
last_seen_emails = set()

# Urgent keywords for email alerts
URGENT_KEYWORDS = [
    'urgent', 'asap', 'emergency', 'critical', 'deadline', 'immediately',
    'time-sensitive', 'action required', 'past due', 'overdue', 'final notice',
    'wire transfer', 'compliance', 'regulatory', 'lawsuit', 'subpoena',
    'security alert', 'breach', 'unauthorized', 'fraud', 'suspicious',
    'payment failed', 'account locked', 'expiring', 'termination',
]


def is_admin(user_id):
    return user_id == ADMIN_ID


def send_admin_message(text):
    try:
        if len(text) <= 4096:
            bot.send_message(ADMIN_ID, text, parse_mode='Markdown')
        else:
            chunks = [text[i:i+4000] for i in range(0, len(text), 4000)]
            for chunk in chunks:
                bot.send_message(ADMIN_ID, chunk)
        return True
    except Exception as e:
        logger.error(f"Failed to send message: {e}")
        return False


def _check_urgency(subject: str, snippet: str) -> tuple:
    """Check if email is urgent. Returns (is_urgent, matched_keywords)."""
    text = f"{subject} {snippet}".lower()
    matched = [kw for kw in URGENT_KEYWORDS if kw in text]
    return (len(matched) > 0, matched)


def _scan_emails_once():
    """Scan for new/urgent emails and alert admin."""
    global last_seen_emails
    try:
        result = subprocess.run(
            ['python3', '/opt/swarm/comms/email_client.py', 'sean_grady', 'list'],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode != 0:
            return

        # Parse email list output
        new_urgent = []
        new_emails = []
        for line in result.stdout.strip().split('\n'):
            line = line.strip()
            if not line or line.startswith('['):
                continue
            # Format: "  From: Subject"
            email_id = hash(line)
            if email_id not in last_seen_emails:
                last_seen_emails.add(email_id)
                is_urgent, keywords = _check_urgency(line, "")
                if is_urgent:
                    new_urgent.append((line, keywords))
                else:
                    new_emails.append(line)

        # Alert for urgent emails
        if new_urgent:
            msg = "🚨 *URGENT EMAIL ALERT*\n\n"
            for email_line, kws in new_urgent:
                msg += f"• {email_line}\n"
                msg += f"  _Flagged: {', '.join(kws)}_\n\n"
            msg += "Reply with `/read` to review or `/dictate` to compose a response."
            send_admin_message(msg)

        # Notify for new non-urgent emails
        if new_emails and len(new_emails) <= 5:
            msg = f"📬 *{len(new_emails)} new email{'s' if len(new_emails) > 1 else ''}*\n\n"
            for email_line in new_emails[:5]:
                msg += f"• {email_line}\n"
            send_admin_message(msg)

    except Exception as e:
        logger.error(f"Email scan error: {e}")


def _email_scanner_loop():
    """Background thread that scans emails periodically."""
    global email_scan_active
    logger.info("Email scanner started")
    while email_scan_active:
        _scan_emails_once()
        time.sleep(email_scan_interval)
    logger.info("Email scanner stopped")


# ── Commands ─────────────────────────────────────────────────────

@bot.message_handler(commands=['start', 'help'])
def send_welcome(message):
    if not is_admin(message.chat.id):
        return
    welcome = (
        "*Calculus AI Command Center*\n\n"
        "*System:*\n"
        "/status — System status\n"
        "/gpu — GPU status\n"
        "/logs — Recent logs\n\n"
        "*Comms:*\n"
        "/sms — Send test SMS\n"
        "/call — Send test call\n"
        "/email — Check inbox summary\n"
        "/send — Send a message (any channel)\n\n"
        "*Email Scanning:*\n"
        "/scan\\_on — Start active email monitoring\n"
        "/scan\\_off — Stop email monitoring\n\n"
        "*Dictation:*\n"
        "/dictate — Dictate an email to send\n"
        "/reply — Reply to the last urgent email\n\n"
        "Or just type a message — I'll route it."
    )
    bot.reply_to(message, welcome, parse_mode='Markdown')


@bot.message_handler(commands=['status'])
def send_status(message):
    if not is_admin(message.chat.id):
        return
    try:
        status = subprocess.getoutput(
            'echo "Host: $(hostname)" && echo "Uptime: $(uptime -p)" && '
            'echo "Memory: $(free -h | grep Mem | awk \'{print $3 "/" $2}\')" && '
            'echo "Disk: $(df -h / | tail -1 | awk \'{print $4 " free"}\')"'
        )
        scan_status = "Active" if email_scan_active else "Inactive"
        bot.reply_to(message,
            f"*Calculus AI Status:*\n```\n{status}\n```\n"
            f"Email Scanner: {scan_status}",
            parse_mode='Markdown')
    except Exception as e:
        bot.reply_to(message, f"Error: {e}")


@bot.message_handler(commands=['gpu'])
def gpu_status(message):
    if not is_admin(message.chat.id):
        return
    try:
        gpu = subprocess.getoutput(
            'ssh swarm-gpu "nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total '
            '--format=csv,noheader" 2>/dev/null || echo "GPU not reachable"'
        )
        bot.reply_to(message, f"*GPU Status:*\n```\n{gpu}\n```", parse_mode='Markdown')
    except Exception as e:
        bot.reply_to(message, f"Error: {e}")


@bot.message_handler(commands=['logs'])
def view_logs(message):
    if not is_admin(message.chat.id):
        return
    try:
        logs = subprocess.getoutput(f'tail -20 {LOG_DIR}/bot.log 2>/dev/null || echo "No logs"')
        bot.reply_to(message, f"*Recent Logs:*\n```\n{logs}\n```", parse_mode='Markdown')
    except Exception as e:
        bot.reply_to(message, f"Error: {e}")


@bot.message_handler(commands=['sms'])
def test_sms(message):
    if not is_admin(message.chat.id):
        return
    try:
        result = subprocess.getoutput('python3 /opt/swarm/comms/send.py "Calculus AI SMS test" sms bunny')
        bot.reply_to(message, f"*SMS Result:*\n```\n{result}\n```", parse_mode='Markdown')
    except Exception as e:
        bot.reply_to(message, f"Error: {e}")


@bot.message_handler(commands=['call'])
def test_call(message):
    if not is_admin(message.chat.id):
        return
    try:
        result = subprocess.getoutput('python3 /opt/swarm/comms/send.py "Calculus AI call test. All systems operational." call bunny')
        bot.reply_to(message, f"*Call Result:*\n```\n{result}\n```", parse_mode='Markdown')
    except Exception as e:
        bot.reply_to(message, f"Error: {e}")


@bot.message_handler(commands=['email'])
def check_email(message):
    if not is_admin(message.chat.id):
        return
    try:
        result = subprocess.run(
            ['python3', '/opt/swarm/comms/email_client.py', 'sean_grady', 'summary'],
            capture_output=True, text=True, timeout=30
        )
        output = result.stdout.strip() or result.stderr.strip() or "No email data available. Connect Gmail first."
        bot.reply_to(message, f"*Inbox Summary:*\n{output}", parse_mode='Markdown')
    except subprocess.TimeoutExpired:
        bot.reply_to(message, "Email check timed out. Gmail may not be connected yet.")
    except Exception as e:
        bot.reply_to(message, f"Error: {e}")


@bot.message_handler(commands=['scan_on'])
def start_email_scan(message):
    if not is_admin(message.chat.id):
        return
    global email_scan_active
    if email_scan_active:
        bot.reply_to(message, "Email scanning is already active.")
        return
    email_scan_active = True
    t = threading.Thread(target=_email_scanner_loop, daemon=True)
    t.start()
    bot.reply_to(message,
        f"*Email scanning activated.*\n"
        f"Checking every {email_scan_interval // 60} minutes.\n"
        f"I'll alert you immediately for urgent emails.",
        parse_mode='Markdown')


@bot.message_handler(commands=['scan_off'])
def stop_email_scan(message):
    if not is_admin(message.chat.id):
        return
    global email_scan_active
    email_scan_active = False
    bot.reply_to(message, "Email scanning stopped.")


@bot.message_handler(commands=['dictate'])
def dictate_email(message):
    """Start email dictation flow."""
    if not is_admin(message.chat.id):
        return
    bot.reply_to(message,
        "*Email Dictation Mode*\n\n"
        "Tell me what to send. Format:\n"
        "`To: recipient@email.com`\n"
        "`Subject: Your subject`\n"
        "`Body: Your message here`\n\n"
        "Or just type naturally:\n"
        "_\"Send Wayne an email about the meeting tomorrow at 3pm\"_",
        parse_mode='Markdown')


@bot.message_handler(commands=['send'])
def send_comms(message):
    """Send a message via any channel."""
    if not is_admin(message.chat.id):
        return
    bot.reply_to(message,
        "*Send a message via any channel:*\n\n"
        "Format: `/send <channel> <contact> <message>`\n\n"
        "Channels: sms, call, whatsapp, email, telegram\n"
        "Contacts: sean, hugo, wayne\n\n"
        "Example:\n"
        "`/send sms wayne Hey, meeting moved to 4pm`\n"
        "`/send email wayne Meeting update Subject goes here`",
        parse_mode='Markdown')


# ── Natural Language Message Handler ─────────────────────────────

@bot.message_handler(func=lambda message: True)
def handle_message(message):
    if not is_admin(message.chat.id):
        return

    text = message.text.strip()
    logger.info(f"Message from admin: {text}")

    # Try to parse as a send command
    text_lower = text.lower()

    # Pattern: "send <contact> <channel> <message>"
    # or "tell <contact> <message>"
    # or "email <contact> about <topic>"
    # or "call <contact> and say <message>"
    contacts = ['sean', 'hugo', 'wayne']
    channels = {
        'sms': 'sms', 'text': 'sms', 'message': 'sms',
        'call': 'call', 'phone': 'call', 'ring': 'call',
        'whatsapp': 'whatsapp', 'wa': 'whatsapp',
        'email': 'email', 'mail': 'email',
    }

    # Check for email dictation pattern
    if any(text_lower.startswith(p) for p in ['to:', 'send email', 'email ', 'send an email', 'draft']):
        status_msg = bot.reply_to(message, "Composing email...")
        try:
            # Parse natural language email
            if 'to:' in text_lower:
                lines = text.split('\n')
                to_addr = subject = body = ""
                for line in lines:
                    if line.lower().startswith('to:'):
                        to_addr = line.split(':', 1)[1].strip()
                    elif line.lower().startswith('subject:'):
                        subject = line.split(':', 1)[1].strip()
                    elif line.lower().startswith('body:'):
                        body = line.split(':', 1)[1].strip()
                if to_addr and subject:
                    result = subprocess.run(
                        ['python3', '/opt/swarm/comms/email_client.py', 'sean_grady', 'send',
                         to_addr, subject, body or '(no body)'],
                        capture_output=True, text=True, timeout=30
                    )
                    bot.edit_message_text(
                        f"Email sent to {to_addr}\nSubject: {subject}",
                        chat_id=message.chat.id, message_id=status_msg.message_id
                    )
                    return
            bot.edit_message_text(
                "I need a recipient and subject. Try:\n"
                "To: someone@email.com\nSubject: Topic\nBody: Message",
                chat_id=message.chat.id, message_id=status_msg.message_id
            )
        except Exception as e:
            bot.edit_message_text(f"Error: {e}", chat_id=message.chat.id, message_id=status_msg.message_id)
        return

    # Check for comms send pattern
    for contact in contacts:
        if contact in text_lower:
            for keyword, channel in channels.items():
                if keyword in text_lower:
                    status_msg = bot.reply_to(message, f"Sending {channel} to {contact}...")
                    try:
                        result = subprocess.run(
                            ['python3', '/opt/swarm/comms/send.py', '--contact', contact, text, channel, 'jack'],
                            capture_output=True, text=True, timeout=30
                        )
                        output = result.stdout.strip() or "Sent."
                        bot.edit_message_text(
                            f"*{channel.upper()} to {contact.title()}:* {output}",
                            chat_id=message.chat.id, message_id=status_msg.message_id,
                            parse_mode='Markdown'
                        )
                    except Exception as e:
                        bot.edit_message_text(f"Error: {e}", chat_id=message.chat.id, message_id=status_msg.message_id)
                    return

    # Check if it's a task for Calculus AI
    if any(kw in text_lower for kw in ['gradient', 'optimizer', 'similarity', 'cosine',
                                         'statistics', 'outlier', 'tensor', 'symbolic',
                                         'compute', 'analyze', 'calculate']):
        status_msg = bot.reply_to(message, "Routing to Calculus AI...")
        # Route to task dispatcher
        try:
            result = subprocess.run(
                ['python3', '-c', f'''
import json
text = """{text}"""
# Simple keyword classification
categories = {{
    "gradient": ["gradient", "norm", "clip", "backprop"],
    "optimizer": ["quantiz", "spars", "entropy", "weight"],
    "similarity": ["similar", "cosine", "jaccard", "knn", "embedding"],
    "statistics": ["statistic", "outlier", "percentile", "iqr", "batch"],
    "symbolic": ["symbolic", "expression", "policy", "rule"],
    "tensor_ops": ["ternary", "hamming", "l1", "dot product"],
}}
for cat, keywords in categories.items():
    if any(kw in text.lower() for kw in keywords):
        print(json.dumps({{"category": cat, "task": text, "status": "queued"}}))
        break
else:
    print(json.dumps({{"category": "general", "task": text, "status": "queued"}}))
'''],
                capture_output=True, text=True, timeout=10
            )
            output = result.stdout.strip() or '{"status": "queued"}'
            parsed = json.loads(output)
            bot.edit_message_text(
                f"*Task Routed*\n"
                f"Category: `{parsed.get('category', 'general')}`\n"
                f"Status: {parsed.get('status', 'queued')}",
                chat_id=message.chat.id, message_id=status_msg.message_id,
                parse_mode='Markdown'
            )
        except Exception as e:
            bot.edit_message_text(f"Task queued. Error details: {e}",
                chat_id=message.chat.id, message_id=status_msg.message_id)
        return

    # Default: acknowledge and suggest actions
    bot.reply_to(message,
        f"Got it. What should I do with this?\n\n"
        f"• `/send sms wayne {text[:50]}` — SMS it\n"
        f"• `/send email wayne {text[:50]}` — Email it\n"
        f"• `/send call wayne {text[:50]}` — Call and say it\n\n"
        f"Or just tell me: _\"text wayne about this\"_",
        parse_mode='Markdown')


if __name__ == '__main__':
    startup_msg = (
        f"*CALCULUS AI BOT ONLINE*\n\n"
        f"Admin: Sean Grady\n"
        f"Time: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n"
        f"Status: All systems operational\n\n"
        f"Type /help for commands."
    )
    send_admin_message(startup_msg)
    logger.info("Calculus AI Telegram bot started")
    bot.infinity_polling()
