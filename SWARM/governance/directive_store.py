"""
Directive Store
===============
SQLite-backed persistence for Signal directives.
Survives session resets — directives are never lost.

DB path: /opt/swarm/data/directives.db (server) or ./directives.db (local fallback)
"""

import json
import os
import sqlite3
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

_DB_PATH = Path(os.getenv("DIRECTIVE_DB_PATH", "/opt/swarm/data/directives.db"))

# Fallback for local dev
if not _DB_PATH.parent.exists():
    _DB_PATH = Path(__file__).parent / "directives.db"

_SCHEMA = """
CREATE TABLE IF NOT EXISTS directives (
    directive_id    TEXT PRIMARY KEY,
    source_number   TEXT NOT NULL,
    raw_text        TEXT NOT NULL,
    summary         TEXT,
    received_at     TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'received',
    repo            TEXT,
    code_path       TEXT,
    commit_hash     TEXT,
    pushed_at       TEXT,
    error           TEXT,
    metadata        TEXT DEFAULT '{}'
);
"""

STATUSES = ("received", "acknowledged", "building", "pushed", "complete", "error")


@dataclass
class DirectiveRecord:
    directive_id: str
    source_number: str
    raw_text: str
    summary: str = ""
    received_at: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    status: str = "received"
    repo: str = ""
    code_path: str = ""
    commit_hash: str = ""
    pushed_at: str = ""
    error: str = ""
    metadata: dict = field(default_factory=dict)


def _conn() -> sqlite3.Connection:
    _DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    con = sqlite3.connect(str(_DB_PATH))
    con.row_factory = sqlite3.Row
    con.execute(_SCHEMA)
    con.commit()
    return con


def save(record: DirectiveRecord) -> None:
    """Insert or replace a directive record."""
    with _conn() as con:
        con.execute(
            """
            INSERT OR REPLACE INTO directives
              (directive_id, source_number, raw_text, summary, received_at,
               status, repo, code_path, commit_hash, pushed_at, error, metadata)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
            """,
            (
                record.directive_id,
                record.source_number,
                record.raw_text,
                record.summary,
                record.received_at,
                record.status,
                record.repo,
                record.code_path,
                record.commit_hash,
                record.pushed_at,
                record.error,
                json.dumps(record.metadata),
            ),
        )


def update_status(
    directive_id: str,
    status: str,
    *,
    commit_hash: str = "",
    code_path: str = "",
    error: str = "",
    pushed_at: str = "",
) -> None:
    """Update status (and optionally commit/path/error) for an existing record."""
    with _conn() as con:
        con.execute(
            """
            UPDATE directives
               SET status=?,
                   commit_hash=CASE WHEN ?!='' THEN ? ELSE commit_hash END,
                   code_path=CASE WHEN ?!='' THEN ? ELSE code_path END,
                   error=CASE WHEN ?!='' THEN ? ELSE error END,
                   pushed_at=CASE WHEN ?!='' THEN ? ELSE pushed_at END
             WHERE directive_id=?
            """,
            (
                status,
                commit_hash, commit_hash,
                code_path, code_path,
                error, error,
                pushed_at, pushed_at,
                directive_id,
            ),
        )


def get(directive_id: str) -> Optional[DirectiveRecord]:
    """Fetch a single directive by ID."""
    with _conn() as con:
        row = con.execute(
            "SELECT * FROM directives WHERE directive_id=?", (directive_id,)
        ).fetchone()
    return _row_to_record(row) if row else None


def list_all(status: Optional[str] = None) -> list[DirectiveRecord]:
    """List all directives, optionally filtered by status."""
    with _conn() as con:
        if status:
            rows = con.execute(
                "SELECT * FROM directives WHERE status=? ORDER BY received_at DESC",
                (status,),
            ).fetchall()
        else:
            rows = con.execute(
                "SELECT * FROM directives ORDER BY received_at DESC"
            ).fetchall()
    return [_row_to_record(r) for r in rows]


def pending() -> list[DirectiveRecord]:
    """Return directives that are received or acknowledged but not yet built/pushed."""
    with _conn() as con:
        rows = con.execute(
            "SELECT * FROM directives WHERE status IN ('received','acknowledged') ORDER BY received_at ASC"
        ).fetchall()
    return [_row_to_record(r) for r in rows]


def summary_table() -> str:
    """Return a human-readable summary table of all directives."""
    records = list_all()
    if not records:
        return "No directives on record."
    lines = [
        f"{'ID':<28}  {'status':<12}  {'commit':<10}  {'received':<25}  summary",
        "-" * 100,
    ]
    for r in records:
        commit = r.commit_hash[:8] if r.commit_hash else "—"
        lines.append(
            f"{r.directive_id:<28}  {r.status:<12}  {commit:<10}  {r.received_at[:19]:<25}  {r.summary[:50]}"
        )
    return "\n".join(lines)


def _row_to_record(row: sqlite3.Row) -> DirectiveRecord:
    return DirectiveRecord(
        directive_id=row["directive_id"],
        source_number=row["source_number"],
        raw_text=row["raw_text"],
        summary=row["summary"] or "",
        received_at=row["received_at"],
        status=row["status"],
        repo=row["repo"] or "",
        code_path=row["code_path"] or "",
        commit_hash=row["commit_hash"] or "",
        pushed_at=row["pushed_at"] or "",
        error=row["error"] or "",
        metadata=json.loads(row["metadata"] or "{}"),
    )
