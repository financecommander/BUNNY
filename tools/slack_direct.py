#!/usr/bin/env python3
"""
Bunny AI — Direct Slack Tool

Standalone Slack interface that bypasses OpenClaw entirely.
Connects directly to the Slack API for send/receive/listen operations.

Usage:
    python slack_direct.py send <channel> <message>
    python slack_direct.py listen [channel]
    python slack_direct.py channels
    python slack_direct.py history <channel> [count]
    python slack_direct.py react <channel> <timestamp> <emoji>
    python slack_direct.py thread <channel> <thread_ts> <message>
    python slack_direct.py users
    python slack_direct.py dm <user_id> <message>
    python slack_direct.py status

Environment:
    SLACK_BOT_TOKEN  — Bot User OAuth Token (xoxb-...)

No OpenClaw, no SWARM, no middleware. Direct Slack API.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from typing import Any, Dict, List, Optional
from urllib.error import HTTPError
from urllib.request import Request, urlopen


# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

SLACK_API = "https://slack.com/api"
TOKEN = os.environ.get("SLACK_BOT_TOKEN", "")


def _check_token():
    if not TOKEN:
        print("ERROR: SLACK_BOT_TOKEN not set in environment")
        print("  export SLACK_BOT_TOKEN=xoxb-...")
        sys.exit(1)


# ---------------------------------------------------------------------------
# Low-level API
# ---------------------------------------------------------------------------

def slack_post(method: str, payload: Dict[str, Any]) -> Dict[str, Any]:
    """POST to Slack API and return parsed JSON response."""
    _check_token()
    url = f"{SLACK_API}/{method}"
    data = json.dumps(payload).encode("utf-8")
    req = Request(url, data=data)
    req.add_header("Authorization", f"Bearer {TOKEN}")
    req.add_header("Content-Type", "application/json; charset=utf-8")
    try:
        with urlopen(req, timeout=30) as resp:
            result = json.loads(resp.read().decode("utf-8"))
    except HTTPError as e:
        body = e.read().decode("utf-8") if e.fp else str(e)
        return {"ok": False, "error": f"HTTP {e.code}: {body}"}
    if not result.get("ok"):
        print(f"Slack API error ({method}): {result.get('error', 'unknown')}")
    return result


def slack_get(method: str, params: Optional[Dict[str, str]] = None) -> Dict[str, Any]:
    """GET from Slack API and return parsed JSON response."""
    _check_token()
    query = ""
    if params:
        query = "&".join(f"{k}={v}" for k, v in params.items())
    url = f"{SLACK_API}/{method}?{query}" if query else f"{SLACK_API}/{method}"
    req = Request(url)
    req.add_header("Authorization", f"Bearer {TOKEN}")
    try:
        with urlopen(req, timeout=30) as resp:
            result = json.loads(resp.read().decode("utf-8"))
    except HTTPError as e:
        body = e.read().decode("utf-8") if e.fp else str(e)
        return {"ok": False, "error": f"HTTP {e.code}: {body}"}
    return result


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

def cmd_send(channel: str, text: str, thread_ts: Optional[str] = None) -> None:
    """Send a message to a channel."""
    payload: Dict[str, Any] = {"channel": channel, "text": text}
    if thread_ts:
        payload["thread_ts"] = thread_ts
    result = slack_post("chat.postMessage", payload)
    if result["ok"]:
        ts = result["message"]["ts"]
        print(f"Sent to #{channel} (ts={ts})")
    else:
        print(f"Failed: {result.get('error')}")


def cmd_channels() -> None:
    """List all accessible channels."""
    result = slack_get("conversations.list", {"types": "public_channel,private_channel", "limit": "100"})
    if not result["ok"]:
        print(f"Error: {result.get('error')}")
        return
    channels = result.get("channels", [])
    print(f"\n{'Channel':<30} {'ID':<15} {'Members':>7} {'Member?':>7}")
    print("-" * 65)
    for ch in sorted(channels, key=lambda c: c["name"]):
        member = "YES" if ch.get("is_member") else ""
        print(f"#{ch['name']:<29} {ch['id']:<15} {ch.get('num_members', '?'):>7} {member:>7}")
    print(f"\nTotal: {len(channels)} channels")


def cmd_history(channel: str, count: int = 10) -> None:
    """Show recent messages in a channel."""
    result = slack_get("conversations.history", {"channel": channel, "limit": str(count)})
    if not result["ok"]:
        print(f"Error: {result.get('error')}")
        return
    messages = result.get("messages", [])
    # Resolve user names
    user_cache: Dict[str, str] = {}
    for msg in reversed(messages):
        user_id = msg.get("user", msg.get("bot_id", "system"))
        if user_id not in user_cache and user_id.startswith("U"):
            uinfo = slack_get("users.info", {"user": user_id})
            if uinfo["ok"]:
                user_cache[user_id] = uinfo["user"].get("real_name", uinfo["user"].get("name", user_id))
            else:
                user_cache[user_id] = user_id
        name = user_cache.get(user_id, user_id)
        ts = time.strftime("%H:%M:%S", time.localtime(float(msg["ts"])))
        text = msg.get("text", "")[:120]
        thread = " [thread]" if msg.get("thread_ts") and msg["thread_ts"] != msg["ts"] else ""
        print(f"[{ts}] {name}: {text}{thread}")


def cmd_listen(channel: Optional[str] = None) -> None:
    """Poll for new messages (simple polling, not WebSocket)."""
    print(f"Listening for messages{f' in {channel}' if channel else ''}... (Ctrl+C to stop)")
    print("-" * 60)
    last_ts = str(time.time())
    while True:
        try:
            params = {"limit": "5", "oldest": last_ts}
            if channel:
                params["channel"] = channel
                result = slack_get("conversations.history", params)
            else:
                # Without a channel, we poll the RTM-style (not available without socket mode)
                print("Note: Specify a channel ID for listening. Polling #general...")
                return

            if result["ok"]:
                messages = result.get("messages", [])
                for msg in reversed(messages):
                    if float(msg["ts"]) > float(last_ts):
                        user = msg.get("user", msg.get("bot_id", "?"))
                        text = msg.get("text", "")[:200]
                        ts_str = time.strftime("%H:%M:%S", time.localtime(float(msg["ts"])))
                        print(f"[{ts_str}] {user}: {text}")
                        last_ts = msg["ts"]

            time.sleep(2)  # Poll every 2 seconds
        except KeyboardInterrupt:
            print("\nStopped listening.")
            break


def cmd_react(channel: str, timestamp: str, emoji: str) -> None:
    """Add a reaction to a message."""
    result = slack_post("reactions.add", {
        "channel": channel,
        "timestamp": timestamp,
        "name": emoji.strip(":")
    })
    if result["ok"]:
        print(f"Reacted with :{emoji.strip(':')}:")
    else:
        print(f"Failed: {result.get('error')}")


def cmd_thread(channel: str, thread_ts: str, text: str) -> None:
    """Reply in a thread."""
    cmd_send(channel, text, thread_ts=thread_ts)


def cmd_users() -> None:
    """List workspace users."""
    result = slack_get("users.list", {"limit": "100"})
    if not result["ok"]:
        print(f"Error: {result.get('error')}")
        return
    members = result.get("members", [])
    print(f"\n{'Name':<25} {'ID':<15} {'Email':<35} {'Bot?':>5}")
    print("-" * 85)
    for u in sorted(members, key=lambda m: m.get("real_name", "")):
        if u.get("deleted"):
            continue
        name = u.get("real_name", u.get("name", "?"))
        email = u.get("profile", {}).get("email", "")
        bot = "BOT" if u.get("is_bot") else ""
        print(f"{name:<25} {u['id']:<15} {email:<35} {bot:>5}")


def cmd_dm(user_id: str, text: str) -> None:
    """Send a direct message to a user."""
    # Open DM channel first
    result = slack_post("conversations.open", {"users": user_id})
    if not result["ok"]:
        print(f"Failed to open DM: {result.get('error')}")
        return
    dm_channel = result["channel"]["id"]
    cmd_send(dm_channel, text)


def cmd_status() -> None:
    """Check bot authentication status."""
    result = slack_post("auth.test", {})
    if result["ok"]:
        print(f"Bot: {result['user']} ({result['user_id']})")
        print(f"Team: {result['team']} ({result['team_id']})")
        print(f"URL: {result['url']}")
        print("Status: AUTHENTICATED")
    else:
        print(f"Auth failed: {result.get('error')}")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Bunny AI — Direct Slack Tool (no OpenClaw)",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__
    )
    sub = parser.add_subparsers(dest="command", help="Command to run")

    # send
    p_send = sub.add_parser("send", help="Send a message")
    p_send.add_argument("channel", help="Channel ID or name")
    p_send.add_argument("message", nargs="+", help="Message text")

    # channels
    sub.add_parser("channels", help="List channels")

    # history
    p_hist = sub.add_parser("history", help="Show channel history")
    p_hist.add_argument("channel", help="Channel ID")
    p_hist.add_argument("count", nargs="?", type=int, default=10, help="Number of messages")

    # listen
    p_listen = sub.add_parser("listen", help="Poll for new messages")
    p_listen.add_argument("channel", nargs="?", help="Channel ID to listen on")

    # react
    p_react = sub.add_parser("react", help="Add reaction to message")
    p_react.add_argument("channel", help="Channel ID")
    p_react.add_argument("timestamp", help="Message timestamp")
    p_react.add_argument("emoji", help="Emoji name (without colons)")

    # thread
    p_thread = sub.add_parser("thread", help="Reply in a thread")
    p_thread.add_argument("channel", help="Channel ID")
    p_thread.add_argument("thread_ts", help="Parent message timestamp")
    p_thread.add_argument("message", nargs="+", help="Reply text")

    # users
    sub.add_parser("users", help="List workspace users")

    # dm
    p_dm = sub.add_parser("dm", help="Send a direct message")
    p_dm.add_argument("user_id", help="User ID")
    p_dm.add_argument("message", nargs="+", help="Message text")

    # status
    sub.add_parser("status", help="Check bot auth status")

    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        return

    if args.command == "send":
        cmd_send(args.channel, " ".join(args.message))
    elif args.command == "channels":
        cmd_channels()
    elif args.command == "history":
        cmd_history(args.channel, args.count)
    elif args.command == "listen":
        cmd_listen(args.channel)
    elif args.command == "react":
        cmd_react(args.channel, args.timestamp, args.emoji)
    elif args.command == "thread":
        cmd_thread(args.channel, args.thread_ts, " ".join(args.message))
    elif args.command == "users":
        cmd_users()
    elif args.command == "dm":
        cmd_dm(args.user_id, " ".join(args.message))
    elif args.command == "status":
        cmd_status()


if __name__ == "__main__":
    main()
