"""
Signal Ingress Daemon
=====================
Polls signal-cli for incoming messages, parses directives,
persists them to directive_store, and triggers autopush when
code is produced.

Run on swarm-mainframe:
    python -m SWARM.comms.signal_ingress

Or as a systemd service (see signal-ingress.service).

Directive detection rules:
  - Starts with "# SWARM-", "# BUNNY-", "# TRITON-"
  - Starts with "Directive:"
  - Contains "SWARM-MF-" or "SWARM-L4-" anywhere in text
"""

import json
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

from dotenv import load_dotenv

load_dotenv(Path(__file__).parent / ".env")

from ..governance.directive_store import DirectiveRecord, get, pending, save, summary_table, update_status

SIGNAL_CLI    = os.getenv("SIGNAL_CLI_PATH", "signal-cli")
SENDER_NUMBER = os.getenv("SIGNAL_SENDER_NUMBER", "")
POLL_INTERVAL = int(os.getenv("SIGNAL_POLL_INTERVAL_S", "10"))

# Pattern: SWARM-L4-UPGRADE-01, BUNNY-MF-CPU-ALLOC-01, etc.
_DIRECTIVE_ID_RE = re.compile(
    r"\b((?:SWARM|BUNNY|TRITON)-[A-Z0-9]+-[A-Z0-9]+-\d+)\b"
)
_DIRECTIVE_PREFIX_RE = re.compile(
    r"^#\s*((?:SWARM|BUNNY|TRITON)-\S+)|^Directive:\s*(.+)",
    re.MULTILINE | re.IGNORECASE,
)


def _receive_messages() -> list[dict]:
    """Call signal-cli receive --json and return parsed message list."""
    if not SENDER_NUMBER:
        print("[signal_ingress] SIGNAL_SENDER_NUMBER not set", flush=True)
        return []
    try:
        result = subprocess.run(
            [SIGNAL_CLI, "-u", SENDER_NUMBER, "receive", "--json",
             "--ignore-attachments", "--timeout", "5"],
            capture_output=True, text=True, timeout=30
        )
        messages = []
        for line in result.stdout.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                messages.append(json.loads(line))
            except json.JSONDecodeError:
                pass
        return messages
    except Exception as e:
        print(f"[signal_ingress] receive error: {e}", flush=True)
        return []


def _extract_directive_id(text: str) -> str:
    """Extract directive ID from message text, or generate a timestamp-based one."""
    m = _DIRECTIVE_ID_RE.search(text)
    if m:
        return m.group(1)
    # If it looks like a directive but has no formal ID, generate one
    if _DIRECTIVE_PREFIX_RE.search(text):
        ts = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
        return f"SWARM-SIGNAL-{ts}"
    return ""


def _extract_summary(text: str) -> str:
    """Pull first meaningful line as summary."""
    for line in text.splitlines():
        line = line.strip().lstrip("#").strip()
        if line:
            return line[:120]
    return text[:80]


def _is_directive(text: str) -> bool:
    return bool(_DIRECTIVE_ID_RE.search(text) or _DIRECTIVE_PREFIX_RE.search(text))


def _parse_repo(directive_id: str, text: str) -> str:
    """Determine target repo from directive ID prefix."""
    prefix = directive_id.split("-")[0].upper()
    mapping = {"SWARM": "super-duper-spork", "BUNNY": "BUNNY", "TRITON": "Triton"}
    return mapping.get(prefix, "super-duper-spork")


def _acknowledge(sender: str, directive_id: str, summary: str) -> None:
    """Send acknowledgement back to sender via Signal."""
    try:
        from .send import send_signal
        msg = f"[BUNNY] Directive received and logged.\nID: {directive_id}\nSummary: {summary}\nStatus: acknowledged — persisted to store."
        send_signal(msg, recipient=sender)
    except Exception as e:
        print(f"[signal_ingress] ack send error: {e}", flush=True)


def process_message(envelope: dict) -> None:
    """Process a single received Signal envelope."""
    # Navigate the signal-cli JSON structure
    msg = envelope.get("envelope", envelope)
    data_msg = msg.get("dataMessage") or msg.get("syncMessage", {}).get("sentMessage", {})

    if not data_msg:
        return

    text = data_msg.get("message", "") or ""
    sender = msg.get("source") or msg.get("sourceNumber") or ""

    if not text or not _is_directive(text):
        return

    directive_id = _extract_directive_id(text)
    if not directive_id:
        return

    # Deduplicate
    existing = get(directive_id)
    if existing:
        print(f"[signal_ingress] duplicate directive {directive_id} — skipping", flush=True)
        return

    summary = _extract_summary(text)
    repo    = _parse_repo(directive_id, text)

    record = DirectiveRecord(
        directive_id=directive_id,
        source_number=sender,
        raw_text=text,
        summary=summary,
        repo=repo,
        status="received",
    )
    save(record)
    print(f"[signal_ingress] stored directive {directive_id}: {summary}", flush=True)

    update_status(directive_id, "acknowledged")
    _acknowledge(sender, directive_id, summary)


def run_once() -> int:
    """Run a single receive pass. Returns number of directives processed."""
    messages = _receive_messages()
    count = 0
    for msg in messages:
        try:
            process_message(msg)
            count += 1
        except Exception as e:
            print(f"[signal_ingress] process error: {e}", flush=True)
    return count


def run_daemon() -> None:
    """Poll Signal continuously."""
    print(f"[signal_ingress] starting — number={SENDER_NUMBER}  interval={POLL_INTERVAL}s", flush=True)

    # Show any pending (unbuilt) directives from prior sessions
    p = pending()
    if p:
        print(f"[signal_ingress] {len(p)} pending directive(s) from prior sessions:", flush=True)
        for r in p:
            print(f"  {r.directive_id}  ({r.received_at[:19]})  {r.summary}", flush=True)

    while True:
        try:
            n = run_once()
            if n:
                print(f"[signal_ingress] processed {n} directive(s)", flush=True)
        except KeyboardInterrupt:
            print("[signal_ingress] stopping", flush=True)
            break
        except Exception as e:
            print(f"[signal_ingress] loop error: {e}", flush=True)
        time.sleep(POLL_INTERVAL)


def print_status() -> None:
    """Print directive store status table."""
    print(summary_table())


if __name__ == "__main__":
    if "--status" in sys.argv:
        print_status()
    elif "--once" in sys.argv:
        n = run_once()
        print(f"Processed {n} directive(s)")
    else:
        run_daemon()
