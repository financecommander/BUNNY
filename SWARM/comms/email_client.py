#!/usr/bin/env python3
"""Calculus AI Email Client — Gmail API with per-user OAuth2 credentials.

Each user's refresh token is stored encrypted (Fernet) in the SQLite
user_profiles table via the `encrypted_creds` field.
"""
import base64
import json
import os
import sqlite3
from email.mime.text import MIMEText
from pathlib import Path
from typing import Dict, List, Optional

from cryptography.fernet import Fernet
from dotenv import load_dotenv

env_path = Path(__file__).parent / ".env"
load_dotenv(env_path)

GMAIL_CLIENT_ID = os.getenv("GMAIL_CLIENT_ID", "")
GMAIL_CLIENT_SECRET = os.getenv("GMAIL_CLIENT_SECRET", "")
EMAIL_ENCRYPTION_KEY = os.getenv("EMAIL_ENCRYPTION_KEY", "")
DB_PATH = os.getenv("BUNNY_DB_PATH", "/opt/bunny-alpha/bunny_memory.db")

SCOPES = [
    "https://www.googleapis.com/auth/gmail.readonly",
    "https://www.googleapis.com/auth/gmail.send",
]


def _get_fernet() -> Fernet:
    if not EMAIL_ENCRYPTION_KEY:
        raise ValueError("EMAIL_ENCRYPTION_KEY not set in .env")
    return Fernet(EMAIL_ENCRYPTION_KEY.encode())


def _encrypt(data: str) -> str:
    return _get_fernet().encrypt(data.encode()).decode()


def _decrypt(data: str) -> str:
    return _get_fernet().decrypt(data.encode()).decode()


def _store_creds(user_id: str, creds_json: str):
    """Store encrypted credentials in user_profiles."""
    encrypted = _encrypt(creds_json)
    conn = sqlite3.connect(DB_PATH, timeout=10)
    try:
        conn.execute(
            "UPDATE user_profiles SET encrypted_creds = ?, email_provider = 'gmail' WHERE user_id = ?",
            (encrypted, user_id),
        )
        conn.commit()
    finally:
        conn.close()


def _load_creds(user_id: str) -> Optional[Dict]:
    """Load and decrypt credentials for a user."""
    conn = sqlite3.connect(DB_PATH, timeout=10)
    try:
        row = conn.execute(
            "SELECT encrypted_creds FROM user_profiles WHERE user_id = ?",
            (user_id,),
        ).fetchone()
        if not row or not row[0]:
            return None
        decrypted = _decrypt(row[0])
        return json.loads(decrypted)
    finally:
        conn.close()


def _get_gmail_service(user_id: str):
    """Build an authenticated Gmail API service for a user."""
    from google.oauth2.credentials import Credentials
    from googleapiclient.discovery import build

    creds_data = _load_creds(user_id)
    if not creds_data:
        raise ValueError(f"No Gmail credentials for user {user_id}. Run connect_gmail() first.")

    creds = Credentials.from_authorized_user_info(creds_data, SCOPES)

    if creds.expired and creds.refresh_token:
        from google.auth.transport.requests import Request
        creds.refresh(Request())
        _store_creds(user_id, creds.to_json())

    return build("gmail", "v1", credentials=creds)


def generate_auth_url() -> str:
    """Generate OAuth2 authorization URL for Gmail."""
    from google_auth_oauthlib.flow import Flow

    flow = Flow.from_client_config(
        {
            "installed": {
                "client_id": GMAIL_CLIENT_ID,
                "client_secret": GMAIL_CLIENT_SECRET,
                "auth_uri": "https://accounts.google.com/o/oauth2/auth",
                "token_uri": "https://oauth2.googleapis.com/token",
            }
        },
        scopes=SCOPES,
        redirect_uri="urn:ietf:wg:oauth:2.0:oob",
    )
    url, _ = flow.authorization_url(prompt="consent", access_type="offline")
    return url


def complete_auth(user_id: str, auth_code: str) -> bool:
    """Complete OAuth2 flow with the authorization code."""
    from google_auth_oauthlib.flow import Flow

    flow = Flow.from_client_config(
        {
            "installed": {
                "client_id": GMAIL_CLIENT_ID,
                "client_secret": GMAIL_CLIENT_SECRET,
                "auth_uri": "https://accounts.google.com/o/oauth2/auth",
                "token_uri": "https://oauth2.googleapis.com/token",
            }
        },
        scopes=SCOPES,
        redirect_uri="urn:ietf:wg:oauth:2.0:oob",
    )
    flow.fetch_token(code=auth_code)
    creds = flow.credentials
    _store_creds(user_id, creds.to_json())
    print(f"[EMAIL] Gmail connected for user {user_id}")
    return True


def list_emails(user_id: str, query: str = "is:unread", max_results: int = 10) -> List[Dict]:
    """List emails matching a query."""
    service = _get_gmail_service(user_id)
    results = service.users().messages().list(
        userId="me", q=query, maxResults=max_results
    ).execute()

    messages = []
    for msg_ref in results.get("messages", []):
        msg = service.users().messages().get(
            userId="me", id=msg_ref["id"], format="metadata",
            metadataHeaders=["From", "Subject", "Date"],
        ).execute()
        headers = {h["name"]: h["value"] for h in msg.get("payload", {}).get("headers", [])}
        messages.append({
            "id": msg["id"],
            "from": headers.get("From", ""),
            "subject": headers.get("Subject", ""),
            "date": headers.get("Date", ""),
            "snippet": msg.get("snippet", ""),
        })
    return messages


def read_email(user_id: str, message_id: str) -> Dict:
    """Read a full email by message ID."""
    service = _get_gmail_service(user_id)
    msg = service.users().messages().get(
        userId="me", id=message_id, format="full"
    ).execute()

    headers = {h["name"]: h["value"] for h in msg.get("payload", {}).get("headers", [])}
    body = ""
    payload = msg.get("payload", {})
    if payload.get("body", {}).get("data"):
        body = base64.urlsafe_b64decode(payload["body"]["data"]).decode("utf-8", errors="replace")
    elif payload.get("parts"):
        for part in payload["parts"]:
            if part.get("mimeType") == "text/plain" and part.get("body", {}).get("data"):
                body = base64.urlsafe_b64decode(part["body"]["data"]).decode("utf-8", errors="replace")
                break

    return {
        "id": msg["id"],
        "from": headers.get("From", ""),
        "to": headers.get("To", ""),
        "subject": headers.get("Subject", ""),
        "date": headers.get("Date", ""),
        "body": body,
    }


def send_email(user_id: str, to: str, subject: str, body: str) -> bool:
    """Send an email via Gmail API."""
    service = _get_gmail_service(user_id)
    message = MIMEText(body)
    message["to"] = to
    message["subject"] = subject
    raw = base64.urlsafe_b64encode(message.as_bytes()).decode()
    service.users().messages().send(
        userId="me", body={"raw": raw}
    ).execute()
    print(f"[EMAIL] Sent to {to} from user {user_id}")
    return True


def summarize_inbox(user_id: str, max_emails: int = 5) -> str:
    """Get a text summary of recent unread emails."""
    emails = list_emails(user_id, query="is:unread", max_results=max_emails)
    if not emails:
        return "No unread emails."
    lines = [f"You have {len(emails)} unread email{'s' if len(emails) > 1 else ''}:\n"]
    for i, e in enumerate(emails, 1):
        lines.append(f"{i}. From: {e['from']}")
        lines.append(f"   Subject: {e['subject']}")
        lines.append(f"   Preview: {e['snippet'][:80]}\n")
    return "\n".join(lines)


if __name__ == "__main__":
    import sys
    if len(sys.argv) < 3:
        print("Usage: email_client.py <user_id> <action> [args...]")
        print("Actions: auth_url, complete_auth <code>, list, read <msg_id>, send <to> <subject> <body>, summary")
        sys.exit(1)

    uid = sys.argv[1]
    action = sys.argv[2]

    if action == "auth_url":
        print(generate_auth_url())
    elif action == "complete_auth":
        complete_auth(uid, sys.argv[3])
    elif action == "list":
        for e in list_emails(uid):
            print(f"  {e['from']}: {e['subject']}")
    elif action == "read":
        email = read_email(uid, sys.argv[3])
        print(f"From: {email['from']}\nSubject: {email['subject']}\n\n{email['body'][:500]}")
    elif action == "send":
        send_email(uid, sys.argv[3], sys.argv[4], sys.argv[5])
    elif action == "summary":
        print(summarize_inbox(uid))
