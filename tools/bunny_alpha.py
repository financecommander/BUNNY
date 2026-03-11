#!/usr/bin/env python3
"""
Bunny Alpha v2.0 — Multi-task Infrastructure Operator

Standalone Slack assistant with real infrastructure execution.
Task queue, concurrent execution, progress reporting.

Architecture:
    Slack Events -> Command Router -> Task Manager -> Tool Executor -> Slack Updates
                                   -> AI Model (chat) -> Slack Reply

Environment:
    SLACK_BOT_TOKEN       — Bot User OAuth Token (xoxb-...)
    SLACK_SIGNING_SECRET  — Signing Secret for request verification
    DEEPSEEK_API_KEY      — DeepSeek API key (primary)
    GROQ_API_KEY          — Groq API key (fallback)
    XAI_API_KEY           — xAI/Grok API key (fallback)
    OLLAMA_URL            — Ollama base URL (local fallback)
    BUNNY_ALPHA_PORT      — Port to listen on (default: 8090)
"""

from __future__ import annotations

import asyncio
import functools
import hashlib
import hmac
import json
import logging
import os
import re
import sqlite3
import time
import uuid
from collections import deque
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, Deque, List, Optional, Tuple

from aiohttp import web, ClientSession, ClientTimeout

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
AI_PORTAL_URL = os.environ.get("SWARM_AI_PORTAL_URL", "http://10.142.0.2:8000").rstrip("/")
AI_PORTAL_TOKEN = os.environ.get("AI_PORTAL_API_KEY", "")
AI_PORTAL_REFRESH = os.environ.get("AI_PORTAL_REFRESH_TOKEN", "")
PORT = int(os.environ.get("BUNNY_ALPHA_PORT", "8090"))
BOT_USER_ID: str = ""

# Active model selection (can be changed via /model command)
_active_provider: str = "deepseek"
_active_model: str = "deepseek-chat"

# Dedup
_seen_events: Dict[str, float] = {}

# HTTP session
_session: Optional[ClientSession] = None

# VM Configuration
VMS = {
    "swarm-mainframe": {"ip": "10.142.0.4", "zone": "us-east1-b", "local": True},
    "swarm-gpu":       {"ip": "10.142.0.6", "zone": "us-east1-b", "local": False},
    "fc-ai-portal":    {"ip": "10.142.0.2", "zone": "us-east1-b", "local": False},
    "calculus-web":    {"ip": "10.142.0.3", "zone": "us-east1-b", "local": False},
}

MAX_CONCURRENT_TASKS = 5
MEMORY_SIZE = 50  # context window size (messages sent to AI)
SUMMARIZE_THRESHOLD = 80  # auto-summarize when channel exceeds this many messages
DB_PATH = os.environ.get("BUNNY_DB_PATH", "/opt/bunny-alpha/bunny_memory.db")


# ---------------------------------------------------------------------------
# Persistent Memory (SQLite-backed)
# ---------------------------------------------------------------------------

def _db_connect() -> sqlite3.Connection:
    """Create a new SQLite connection (one per call for thread safety)."""
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def _init_db():
    """Create database tables if they don't exist."""
    conn = _db_connect()
    try:
        conn.executescript("""
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                thread_ts TEXT,
                user_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                token_estimate INTEGER DEFAULT 0,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_channel ON messages(channel_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(channel_id, thread_ts, created_at);

            CREATE TABLE IF NOT EXISTS memory_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scope_type TEXT NOT NULL,
                scope_id TEXT NOT NULL,
                summary TEXT NOT NULL,
                message_count INTEGER DEFAULT 0,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_summaries_scope ON memory_summaries(scope_type, scope_id);

            CREATE TABLE IF NOT EXISTS task_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                channel_id TEXT,
                thread_ts TEXT,
                request TEXT,
                status TEXT DEFAULT 'pending',
                result_summary TEXT,
                created_at REAL NOT NULL,
                completed_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_task_runs_task ON task_runs(task_id);
            CREATE INDEX IF NOT EXISTS idx_task_runs_channel ON task_runs(channel_id, created_at);

            CREATE TABLE IF NOT EXISTS preferences (
                user_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT,
                updated_at REAL NOT NULL,
                PRIMARY KEY (user_id, key)
            );
        """)
        conn.commit()
        log.info(f"Persistent memory initialized: {DB_PATH}")
    finally:
        conn.close()


class PersistentMemory:
    """SQLite-backed conversation memory that survives restarts.

    All public methods are async (use asyncio.to_thread for DB ops).
    The context window (messages sent to AI) is capped at MEMORY_SIZE,
    but all messages are stored persistently.
    """

    def __init__(self, context_window: int = MEMORY_SIZE):
        self.context_window = context_window

    # -- helpers --

    @staticmethod
    def _estimate_tokens(text: str) -> int:
        """Rough token estimate (~4 chars per token)."""
        return max(1, len(text) // 4)

    @staticmethod
    def _run_sync(fn, *args):
        """Run a sync DB function in the thread pool."""
        return asyncio.to_thread(fn, *args)

    # -- message storage --

    async def add(self, channel: str, role: str, content: str,
                  thread_ts: str = None, user_id: str = None):
        """Store a message persistently."""
        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO messages (channel_id, thread_ts, user_id, role, content, token_estimate, created_at) "
                    "VALUES (?, ?, ?, ?, ?, ?, ?)",
                    (channel, thread_ts, user_id, role, content,
                     self._estimate_tokens(content), time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await self._run_sync(_insert)

    async def get_history(self, channel: str, thread_ts: str = None,
                          limit: int = None) -> List[Dict[str, str]]:
        """Return recent message history for AI context."""
        n = limit or self.context_window

        def _query():
            conn = _db_connect()
            try:
                if thread_ts:
                    rows = conn.execute(
                        "SELECT role, content FROM messages "
                        "WHERE channel_id = ? AND thread_ts = ? "
                        "ORDER BY created_at DESC LIMIT ?",
                        (channel, thread_ts, n),
                    ).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT role, content FROM messages "
                        "WHERE channel_id = ? "
                        "ORDER BY created_at DESC LIMIT ?",
                        (channel, n),
                    ).fetchall()
                # Reverse to chronological order
                return [{"role": r["role"], "content": r["content"]} for r in reversed(rows)]
            finally:
                conn.close()
        return await self._run_sync(_query)

    async def clear(self, channel: str, thread_ts: str = None):
        """Clear messages for a channel or thread."""
        def _delete():
            conn = _db_connect()
            try:
                if thread_ts:
                    conn.execute(
                        "DELETE FROM messages WHERE channel_id = ? AND thread_ts = ?",
                        (channel, thread_ts),
                    )
                else:
                    conn.execute("DELETE FROM messages WHERE channel_id = ?", (channel,))
                    conn.execute(
                        "DELETE FROM memory_summaries WHERE scope_type = 'channel' AND scope_id = ?",
                        (channel,),
                    )
                conn.commit()
            finally:
                conn.close()
        await self._run_sync(_delete)

    async def clear_all(self):
        """Clear all messages and summaries."""
        def _delete():
            conn = _db_connect()
            try:
                conn.execute("DELETE FROM messages")
                conn.execute("DELETE FROM memory_summaries")
                conn.commit()
            finally:
                conn.close()
        await self._run_sync(_delete)

    async def stats(self) -> Dict[str, Any]:
        """Return comprehensive memory stats."""
        def _query():
            conn = _db_connect()
            try:
                # Per-channel counts
                channels = {}
                for row in conn.execute(
                    "SELECT channel_id, COUNT(*) as cnt FROM messages GROUP BY channel_id"
                ).fetchall():
                    channels[row["channel_id"]] = row["cnt"]

                # Total messages
                total = conn.execute("SELECT COUNT(*) FROM messages").fetchone()[0]

                # DB size
                db_size = os.path.getsize(DB_PATH) if os.path.exists(DB_PATH) else 0

                # Summary count
                summaries = conn.execute("SELECT COUNT(*) FROM memory_summaries").fetchone()[0]

                # Task runs
                task_count = conn.execute("SELECT COUNT(*) FROM task_runs").fetchone()[0]

                # Preferences
                pref_count = conn.execute("SELECT COUNT(*) FROM preferences").fetchone()[0]

                # Oldest message
                oldest = conn.execute(
                    "SELECT MIN(created_at) FROM messages"
                ).fetchone()[0]

                return {
                    "channels": channels,
                    "total_messages": total,
                    "summaries": summaries,
                    "task_runs": task_count,
                    "preferences": pref_count,
                    "db_size_bytes": db_size,
                    "oldest_message": oldest,
                }
            finally:
                conn.close()
        return await self._run_sync(_query)

    # -- summaries --

    async def get_summary(self, scope_type: str, scope_id: str) -> Optional[str]:
        """Get the latest summary for a scope (channel, thread, etc.)."""
        def _query():
            conn = _db_connect()
            try:
                row = conn.execute(
                    "SELECT summary FROM memory_summaries "
                    "WHERE scope_type = ? AND scope_id = ? "
                    "ORDER BY updated_at DESC LIMIT 1",
                    (scope_type, scope_id),
                ).fetchone()
                return row["summary"] if row else None
            finally:
                conn.close()
        return await self._run_sync(_query)

    async def save_summary(self, scope_type: str, scope_id: str,
                           summary: str, message_count: int = 0):
        """Save or update a summary for a scope."""
        def _upsert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO memory_summaries (scope_type, scope_id, summary, message_count, updated_at) "
                    "VALUES (?, ?, ?, ?, ?)",
                    (scope_type, scope_id, summary, message_count, time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await self._run_sync(_upsert)

    async def auto_summarize_if_needed(self, channel: str):
        """If a channel has too many messages, summarize older ones."""
        def _check_and_summarize():
            conn = _db_connect()
            try:
                count = conn.execute(
                    "SELECT COUNT(*) FROM messages WHERE channel_id = ?", (channel,)
                ).fetchone()[0]
                if count <= SUMMARIZE_THRESHOLD:
                    return None  # No summarization needed

                # Get oldest messages beyond the context window
                keep = self.context_window
                rows = conn.execute(
                    "SELECT id, role, content FROM messages "
                    "WHERE channel_id = ? ORDER BY created_at ASC LIMIT ?",
                    (channel, count - keep),
                ).fetchall()

                if not rows:
                    return None

                # Build text for summarization
                text_parts = []
                ids_to_remove = []
                for r in rows:
                    text_parts.append(f"{r['role']}: {r['content'][:200]}")
                    ids_to_remove.append(r["id"])

                return {
                    "text": "\n".join(text_parts),
                    "ids": ids_to_remove,
                    "count": len(ids_to_remove),
                }
            finally:
                conn.close()

        result = await self._run_sync(_check_and_summarize)
        if not result:
            return None
        return result  # Caller will summarize via AI and call complete_summarize

    async def complete_summarize(self, channel: str, summary: str,
                                 message_ids: List[int]):
        """After AI generates summary, store it and remove old messages."""
        def _do():
            conn = _db_connect()
            try:
                # Save summary
                conn.execute(
                    "INSERT INTO memory_summaries (scope_type, scope_id, summary, message_count, updated_at) "
                    "VALUES (?, ?, ?, ?, ?)",
                    ("channel", channel, summary, len(message_ids), time.time()),
                )
                # Remove old messages
                placeholders = ",".join("?" * len(message_ids))
                conn.execute(
                    f"DELETE FROM messages WHERE id IN ({placeholders})",
                    message_ids,
                )
                conn.commit()
                log.info(f"Summarized {len(message_ids)} messages for channel {channel}")
            finally:
                conn.close()
        await self._run_sync(_do)

    # -- task runs --

    async def log_task(self, task_id: str, channel: str, thread_ts: str,
                       request: str, status: str = "pending"):
        """Log a task run."""
        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO task_runs (task_id, channel_id, thread_ts, request, status, created_at) "
                    "VALUES (?, ?, ?, ?, ?, ?)",
                    (task_id, channel, thread_ts, request, status, time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await self._run_sync(_insert)

    async def update_task(self, task_id: str, status: str, result_summary: str = None):
        """Update a task run status."""
        def _update():
            conn = _db_connect()
            try:
                if result_summary:
                    conn.execute(
                        "UPDATE task_runs SET status = ?, result_summary = ?, completed_at = ? "
                        "WHERE task_id = ?",
                        (status, result_summary, time.time(), task_id),
                    )
                else:
                    conn.execute(
                        "UPDATE task_runs SET status = ? WHERE task_id = ?",
                        (status, task_id),
                    )
                conn.commit()
            finally:
                conn.close()
        await self._run_sync(_update)

    async def get_recent_tasks(self, channel: str = None, limit: int = 10) -> List[Dict]:
        """Get recent task runs."""
        def _query():
            conn = _db_connect()
            try:
                if channel:
                    rows = conn.execute(
                        "SELECT * FROM task_runs WHERE channel_id = ? "
                        "ORDER BY created_at DESC LIMIT ?",
                        (channel, limit),
                    ).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT * FROM task_runs ORDER BY created_at DESC LIMIT ?",
                        (limit,),
                    ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await self._run_sync(_query)

    # -- preferences --

    async def set_preference(self, user_id: str, key: str, value: str):
        """Set a user preference."""
        def _upsert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO preferences (user_id, key, value, updated_at) "
                    "VALUES (?, ?, ?, ?) "
                    "ON CONFLICT(user_id, key) DO UPDATE SET value = ?, updated_at = ?",
                    (user_id, key, value, time.time(), value, time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await self._run_sync(_upsert)

    async def get_preference(self, user_id: str, key: str) -> Optional[str]:
        """Get a user preference."""
        def _query():
            conn = _db_connect()
            try:
                row = conn.execute(
                    "SELECT value FROM preferences WHERE user_id = ? AND key = ?",
                    (user_id, key),
                ).fetchone()
                return row["value"] if row else None
            finally:
                conn.close()
        return await self._run_sync(_query)

    async def get_all_preferences(self, user_id: str) -> Dict[str, str]:
        """Get all preferences for a user."""
        def _query():
            conn = _db_connect()
            try:
                rows = conn.execute(
                    "SELECT key, value FROM preferences WHERE user_id = ?",
                    (user_id,),
                ).fetchall()
                return {r["key"]: r["value"] for r in rows}
            finally:
                conn.close()
        return await self._run_sync(_query)

    # -- search / knowledge base --

    async def search_messages(self, query: str, channel: str = None,
                              limit: int = 20) -> List[Dict]:
        """Search message history by content."""
        def _query_db():
            conn = _db_connect()
            try:
                pattern = f"%{query}%"
                if channel:
                    rows = conn.execute(
                        "SELECT channel_id, role, content, created_at FROM messages "
                        "WHERE channel_id = ? AND content LIKE ? "
                        "ORDER BY created_at DESC LIMIT ?",
                        (channel, pattern, limit),
                    ).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT channel_id, role, content, created_at FROM messages "
                        "WHERE content LIKE ? "
                        "ORDER BY created_at DESC LIMIT ?",
                        (pattern, limit),
                    ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await self._run_sync(_query_db)


# Initialize persistent memory
_init_db()
memory = PersistentMemory()


# ---------------------------------------------------------------------------
# Bunny Alpha System Prompt
# ---------------------------------------------------------------------------

BUNNY_ALPHA_PROMPT = """You are Bunny Alpha \u2014 Sean's personal AI assistant at Bunny AI (Calculus Holdings).

Friendly, helpful, concise. You have FULL infrastructure access and follow all of Sean's commands.

You can execute real commands on infrastructure. When Sean asks you to DO something
(check status, restart services, run commands, deploy, etc.), respond with executable
commands using this format:

[EXECUTE]
{"tool": "shell", "host": "swarm-mainframe", "cmd": "docker ps --format 'table {{.Names}}\\t{{.Status}}'"}
[/EXECUTE]

Available tools:
- shell: Run shell command. Args: host (vm name or "local"), cmd
- docker: Docker command. Args: host, cmd (e.g. "ps", "logs swarm", "restart swarm")
- ollama: Query Ollama model. Args: model, prompt
- http: HTTP request. Args: url, method (GET/POST), body (optional)
- image: Generate an image. Args: prompt (description of image to generate)

Available hosts: swarm-mainframe (local), swarm-gpu, fc-ai-portal, calculus-web

For multiple tasks, include multiple commands in one [EXECUTE] block \u2014 they run concurrently:
[EXECUTE]
{"tool": "shell", "host": "swarm-mainframe", "cmd": "df -h /"}
{"tool": "shell", "host": "swarm-gpu", "cmd": "nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv"}
{"tool": "shell", "host": "swarm-gpu", "cmd": "df -h /"}
[/EXECUTE]

Rules:
- If it's just a question or chat, respond normally (no [EXECUTE])
- If Sean wants something DONE, use [EXECUTE] commands
- Be concise. No disclaimers. Just do it.
- After commands execute, you'll get results to summarize

You are Bunny Alpha. Friendly. Capable. Always ready."""


# ---------------------------------------------------------------------------
# Task Manager
# ---------------------------------------------------------------------------

class TaskStatus(Enum):
    QUEUED = "queued"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


@dataclass
class Task:
    task_id: str
    tool: str
    host: str
    cmd: str
    status: TaskStatus = TaskStatus.QUEUED
    result: Optional[str] = None
    error: Optional[str] = None
    created_at: float = field(default_factory=time.time)
    started_at: Optional[float] = None
    completed_at: Optional[float] = None
    channel: str = ""
    thread_ts: str = ""

    @property
    def duration(self) -> Optional[float]:
        if self.started_at and self.completed_at:
            return round(self.completed_at - self.started_at, 1)
        return None

    @property
    def short_id(self) -> str:
        return self.task_id[:6]


class TaskManager:
    """Manages concurrent task execution with progress reporting."""

    def __init__(self):
        self.tasks: Dict[str, Task] = {}
        self.active_count = 0
        self._lock = asyncio.Lock()
        # Track task groups (multiple tasks from one user message)
        self.groups: Dict[str, List[str]] = {}  # group_id -> [task_ids]

    def create_task(self, tool: str, host: str, cmd: str,
                    channel: str = "", thread_ts: str = "",
                    group_id: Optional[str] = None) -> Task:
        """Create and register a new task."""
        task_id = uuid.uuid4().hex[:8]
        task = Task(
            task_id=task_id,
            tool=tool,
            host=host,
            cmd=cmd,
            channel=channel,
            thread_ts=thread_ts,
        )
        self.tasks[task_id] = task

        if group_id:
            if group_id not in self.groups:
                self.groups[group_id] = []
            self.groups[group_id].append(task_id)

        log.info(f"Task {task.short_id} created: {tool}@{host} -> {cmd[:60]}")
        return task

    async def execute_group(self, group_id: str, channel: str, thread_ts: str) -> List[Task]:
        """Execute all tasks in a group concurrently."""
        task_ids = self.groups.get(group_id, [])
        if not task_ids:
            return []

        tasks = [self.tasks[tid] for tid in task_ids]
        total = len(tasks)

        # Post initial status
        task_list = "\n".join(
            f"\u2022 `{t.tool}@{t.host}`: `{t.cmd[:50]}`" for t in tasks
        )
        await post_message(
            f":rocket: *Running {total} task{'s' if total > 1 else ''}...*\n{task_list}",
            channel, thread_ts,
        )

        # Execute all concurrently
        results = await asyncio.gather(
            *[self._run_task(t) for t in tasks],
            return_exceptions=True,
        )

        # Handle any exceptions from gather
        for i, result in enumerate(results):
            if isinstance(result, Exception):
                tasks[i].status = TaskStatus.FAILED
                tasks[i].error = str(result)
                tasks[i].completed_at = time.time()

        return tasks

    async def _run_task(self, task: Task) -> Task:
        """Execute a single task."""
        task.status = TaskStatus.RUNNING
        task.started_at = time.time()

        try:
            result = await tool_executor.execute(task.tool, task.host, task.cmd)
            task.result = result
            task.status = TaskStatus.COMPLETED
        except Exception as e:
            task.error = str(e)
            task.status = TaskStatus.FAILED
            log.error(f"Task {task.short_id} failed: {e}")
        finally:
            task.completed_at = time.time()

        return task

    def cancel_task(self, task_id: str) -> bool:
        """Cancel a queued task."""
        task = self.tasks.get(task_id)
        if task and task.status == TaskStatus.QUEUED:
            task.status = TaskStatus.CANCELLED
            return True
        return False

    def get_active_tasks(self) -> List[Task]:
        return [t for t in self.tasks.values() if t.status == TaskStatus.RUNNING]

    def get_recent_tasks(self, limit: int = 10) -> List[Task]:
        return sorted(
            self.tasks.values(),
            key=lambda t: t.created_at,
            reverse=True,
        )[:limit]

    def cleanup_old(self, max_age: float = 3600):
        """Remove tasks older than max_age seconds."""
        now = time.time()
        old_ids = [
            tid for tid, t in self.tasks.items()
            if now - t.created_at > max_age
        ]
        for tid in old_ids:
            del self.tasks[tid]
        # Clean group refs
        for gid in list(self.groups.keys()):
            self.groups[gid] = [
                tid for tid in self.groups[gid] if tid in self.tasks
            ]
            if not self.groups[gid]:
                del self.groups[gid]


# ---------------------------------------------------------------------------
# Tool Executor
# ---------------------------------------------------------------------------

class ToolExecutor:
    """Executes infrastructure commands across VMs."""

    async def execute(self, tool: str, host: str, cmd: str) -> str:
        """Route execution to the right handler."""
        handlers = {
            "shell": self.exec_shell,
            "docker": self.exec_docker,
            "ollama": self.exec_ollama,
            "http": self.exec_http,
            "image": self.exec_image_gen,
        }
        handler = handlers.get(tool)
        if not handler:
            raise ValueError(f"Unknown tool: {tool}")
        return await handler(host, cmd)

    async def exec_shell(self, host: str, cmd: str) -> str:
        """Run shell command on a host."""
        vm = VMS.get(host)
        if not vm:
            # Try matching partial names
            for name, info in VMS.items():
                if host in name or name in host:
                    vm = info
                    host = name
                    break
            if not vm:
                raise ValueError(f"Unknown host: {host}. Available: {', '.join(VMS.keys())}")

        if vm.get("local"):
            return await self._local_exec(cmd)
        else:
            return await self._ssh_exec(host, vm, cmd)

    async def _local_exec(self, cmd: str) -> str:
        """Execute command locally."""
        try:
            proc = await asyncio.create_subprocess_shell(
                cmd,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=60)
            output = stdout.decode("utf-8", errors="replace").strip()
            err = stderr.decode("utf-8", errors="replace").strip()

            if proc.returncode != 0:
                return f"[exit {proc.returncode}]\n{output}\n{err}".strip()
            return output or "(no output)"
        except asyncio.TimeoutError:
            return "[ERROR] Command timed out (60s)"
        except Exception as e:
            return f"[ERROR] {e}"

    async def _ssh_exec(self, host: str, vm: Dict, cmd: str) -> str:
        """Execute command on remote VM via gcloud SSH."""
        zone = vm.get("zone", "us-east1-b")
        try:
            proc = await asyncio.create_subprocess_exec(
                "gcloud", "compute", "ssh", host,
                f"--zone={zone}",
                "--internal-ip",
                f"--command={cmd}",
                "--quiet",
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=90)
            output = stdout.decode("utf-8", errors="replace").strip()
            err = stderr.decode("utf-8", errors="replace").strip()

            if proc.returncode != 0:
                # Filter out SSH warnings
                err_lines = [
                    l for l in err.split("\n")
                    if not l.startswith("Warning:") and not l.startswith("WARNING:")
                ]
                err_clean = "\n".join(err_lines).strip()
                return f"[exit {proc.returncode}]\n{output}\n{err_clean}".strip()
            return output or "(no output)"
        except asyncio.TimeoutError:
            return f"[ERROR] SSH to {host} timed out (90s)"
        except Exception as e:
            return f"[ERROR] SSH to {host}: {e}"

    async def exec_docker(self, host: str, cmd: str) -> str:
        """Run docker command on a host."""
        # Prepend 'docker' if not already there
        if not cmd.strip().startswith("docker"):
            cmd = f"docker {cmd}"
        return await self.exec_shell(host, cmd)

    async def exec_ollama(self, host: str, cmd: str) -> str:
        """Query Ollama model. cmd format: 'model_name: prompt' or just 'prompt'."""
        url = OLLAMA_URL
        if not url:
            # Default to swarm-gpu
            gpu = VMS.get("swarm-gpu", {})
            url = f"http://{gpu.get('ip', '10.142.0.6')}:11434"

        # Parse model and prompt
        if ":" in cmd and not cmd.startswith("/"):
            model, prompt = cmd.split(":", 1)
            model = model.strip()
            prompt = prompt.strip()
        else:
            model = "qwen2.5-coder:7b"
            prompt = cmd

        try:
            async with _session.post(
                f"{url}/api/generate",
                json={"model": model, "prompt": prompt, "stream": False},
                timeout=ClientTimeout(total=120),
            ) as resp:
                data = await resp.json()
                return data.get("response", str(data))
        except Exception as e:
            return f"[ERROR] Ollama: {e}"

    async def exec_http(self, host: str, cmd: str) -> str:
        """Make HTTP request. cmd is URL, or 'METHOD URL [body]'."""
        parts = cmd.strip().split(None, 2)
        if len(parts) >= 2 and parts[0].upper() in ("GET", "POST", "PUT", "DELETE", "PATCH"):
            method = parts[0].upper()
            url = parts[1]
            body = parts[2] if len(parts) > 2 else None
        else:
            method = "GET"
            url = parts[0] if parts else cmd
            body = None

        try:
            kwargs: Dict[str, Any] = {"timeout": ClientTimeout(total=30)}
            if body:
                try:
                    kwargs["json"] = json.loads(body)
                except json.JSONDecodeError:
                    kwargs["data"] = body

            async with _session.request(method, url, **kwargs) as resp:
                text = await resp.text()
                if len(text) > 2000:
                    text = text[:2000] + "\n...(truncated)"
                return f"[{resp.status}] {text}"
        except Exception as e:
            return f"[ERROR] HTTP: {e}"

    async def exec_image_gen(self, host: str, cmd: str) -> str:
        """Generate an image using xAI Grok image generation."""
        if not XAI_API_KEY:
            return "[ERROR] XAI_API_KEY not set — cannot generate images"
        try:
            async with _session.post(
                "https://api.x.ai/v1/images/generations",
                headers={
                    "Authorization": f"Bearer {XAI_API_KEY}",
                    "Content-Type": "application/json",
                },
                json={
                    "model": "grok-2-image",
                    "prompt": cmd,
                    "n": 1,
                    "response_format": "url",
                },
                timeout=ClientTimeout(total=120),
            ) as resp:
                data = await resp.json()
                if "data" in data and data["data"]:
                    image_url = data["data"][0].get("url", "")
                    if image_url:
                        return f"IMAGE_URL:{image_url}"
                    return "[ERROR] No image URL in response"
                error = data.get("error", {}).get("message", str(data))
                return f"[ERROR] Image generation: {error}"
        except Exception as e:
            return f"[ERROR] Image generation: {e}"


# Singleton instances
task_manager = TaskManager()
tool_executor = ToolExecutor()


# ---------------------------------------------------------------------------
# AI Model Providers
# ---------------------------------------------------------------------------

def _build_messages(system: str, history: List[Dict[str, str]], prompt: str) -> List[Dict[str, str]]:
    """Build message array: system + history + current prompt."""
    messages = [{"role": "system", "content": system}]
    messages.extend(history)
    messages.append({"role": "user", "content": prompt})
    return messages


async def query_deepseek(prompt: str, system: str, history: Optional[List[Dict]] = None) -> Optional[str]:
    """Query DeepSeek API with conversation history."""
    if not DEEPSEEK_API_KEY:
        return None
    try:
        messages = _build_messages(system, history or [], prompt)
        async with _session.post(
            "https://api.deepseek.com/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {DEEPSEEK_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "deepseek-chat",
                "messages": messages,
                "max_tokens": 2000,
                "temperature": 0.7,
            },
            timeout=ClientTimeout(total=60),
        ) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"]["content"]
            log.warning(f"DeepSeek error: {data}")
            return None
    except Exception as e:
        log.warning(f"DeepSeek failed: {e}")
        return None


async def query_groq(prompt: str, system: str, history: Optional[List[Dict]] = None) -> Optional[str]:
    """Query Groq API with conversation history."""
    if not GROQ_API_KEY:
        return None
    try:
        messages = _build_messages(system, history or [], prompt)
        async with _session.post(
            "https://api.groq.com/openai/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {GROQ_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "meta-llama/llama-4-scout-17b-16e-instruct",
                "messages": messages,
                "max_tokens": 2000,
                "temperature": 0.7,
            },
            timeout=ClientTimeout(total=60),
        ) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"]["content"]
            log.warning(f"Groq error: {data}")
            return None
    except Exception as e:
        log.warning(f"Groq failed: {e}")
        return None


async def query_xai(prompt: str, system: str, history: Optional[List[Dict]] = None) -> Optional[str]:
    """Query xAI/Grok API with conversation history."""
    if not XAI_API_KEY:
        return None
    try:
        messages = _build_messages(system, history or [], prompt)
        async with _session.post(
            "https://api.x.ai/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {XAI_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "grok-3-fast",
                "messages": messages,
                "max_tokens": 2000,
                "temperature": 0.7,
            },
            timeout=ClientTimeout(total=60),
        ) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"]["content"]
            log.warning(f"xAI error: {data}")
            return None
    except Exception as e:
        log.warning(f"xAI failed: {e}")
        return None


async def query_ollama_chat(prompt: str, system: str, history: Optional[List[Dict]] = None) -> Optional[str]:
    """Query local Ollama instance with conversation history."""
    if not OLLAMA_URL:
        return None
    try:
        messages = _build_messages(system, history or [], prompt)
        async with _session.post(
            f"{OLLAMA_URL}/api/chat",
            json={
                "model": "qwen2.5-coder:7b",
                "messages": messages,
                "stream": False,
            },
            timeout=ClientTimeout(total=120),
        ) as resp:
            data = await resp.json()
            if "message" in data:
                return data["message"].get("content")
            log.warning(f"Ollama error: {data}")
            return None
    except Exception as e:
        log.warning(f"Ollama failed: {e}")
        return None


# ---------------------------------------------------------------------------
# AI Portal Provider (access to ALL models)
# ---------------------------------------------------------------------------

# Full model catalog from AI Portal
PORTAL_MODELS = {
    # OpenAI
    "gpt-5.2":       {"provider": "openai",  "name": "GPT-5.2"},
    "gpt-5":         {"provider": "openai",  "name": "GPT-5"},
    "gpt-4.1":       {"provider": "openai",  "name": "GPT-4.1"},
    "gpt-4.1-mini":  {"provider": "openai",  "name": "GPT-4.1 Mini"},
    "gpt-4.1-nano":  {"provider": "openai",  "name": "GPT-4.1 Nano"},
    "o3-mini":       {"provider": "openai",  "name": "o3-mini"},
    # Anthropic
    "claude-opus-4-6":              {"provider": "anthropic", "name": "Claude Opus 4.6"},
    "claude-sonnet-4-6":            {"provider": "anthropic", "name": "Claude Sonnet 4.6"},
    "claude-opus-4-5":              {"provider": "anthropic", "name": "Claude Opus 4.5"},
    "claude-sonnet-4-5-20250929":   {"provider": "anthropic", "name": "Claude Sonnet 4.5"},
    "claude-haiku-4-5-20251001":    {"provider": "anthropic", "name": "Claude Haiku 4.5"},
    # Google
    "gemini-3.1-pro-preview":  {"provider": "google",  "name": "Gemini 3.1 Pro"},
    "gemini-3-flash-preview":  {"provider": "google",  "name": "Gemini 3 Flash"},
    "gemini-2.5-pro":          {"provider": "google",  "name": "Gemini 2.5 Pro"},
    "gemini-2.5-flash":        {"provider": "google",  "name": "Gemini 2.5 Flash"},
    # xAI
    "grok-4":        {"provider": "grok",    "name": "Grok 4"},
    "grok-4-1-fast": {"provider": "grok",    "name": "Grok 4.1 Fast"},
    "grok-3":        {"provider": "grok",    "name": "Grok 3"},
    # DeepSeek
    "deepseek-reasoner": {"provider": "deepseek", "name": "DeepSeek R1"},
    "deepseek-chat":     {"provider": "deepseek", "name": "DeepSeek V3.2"},
    # Mistral
    "mistral-large-latest":  {"provider": "mistral", "name": "Mistral Large 3"},
    "mistral-medium-latest": {"provider": "mistral", "name": "Mistral Medium 3"},
    # Groq
    "meta-llama/llama-4-maverick-17b-128e-instruct": {"provider": "groq", "name": "Llama 4 Maverick"},
    "meta-llama/llama-4-scout-17b-16e-instruct":     {"provider": "groq", "name": "Llama 4 Scout"},
}

# Short aliases for convenience
MODEL_ALIASES = {
    "gpt5": "gpt-5.2", "gpt": "gpt-5.2",
    "claude": "claude-sonnet-4-6", "opus": "claude-opus-4-6", "sonnet": "claude-sonnet-4-6", "haiku": "claude-haiku-4-5-20251001",
    "gemini": "gemini-3.1-pro-preview", "flash": "gemini-3-flash-preview",
    "grok": "grok-4", "grok4": "grok-4",
    "deepseek": "deepseek-chat", "r1": "deepseek-reasoner",
    "mistral": "mistral-large-latest",
    "llama": "meta-llama/llama-4-maverick-17b-128e-instruct", "maverick": "meta-llama/llama-4-maverick-17b-128e-instruct",
    "scout": "meta-llama/llama-4-scout-17b-16e-instruct",
}


async def _refresh_portal_token():
    """Refresh the AI Portal JWT token."""
    global AI_PORTAL_TOKEN
    if not AI_PORTAL_REFRESH:
        return False
    try:
        async with _session.post(
            f"{AI_PORTAL_URL}/auth/refresh",
            json={"refresh_token": AI_PORTAL_REFRESH},
            timeout=ClientTimeout(total=10),
        ) as resp:
            data = await resp.json()
            if "access_token" in data:
                AI_PORTAL_TOKEN = data["access_token"]
                log.info("AI Portal token refreshed")
                return True
            log.warning(f"Token refresh failed: {data}")
            return False
    except Exception as e:
        log.warning(f"Token refresh error: {e}")
        return False


async def query_portal(prompt: str, system: str, history: Optional[List[Dict]] = None,
                       provider: Optional[str] = None, model: Optional[str] = None) -> Optional[str]:
    """Query AI Portal — routes to any model across all providers."""
    global AI_PORTAL_TOKEN
    if not AI_PORTAL_URL or not AI_PORTAL_TOKEN:
        return None

    use_provider = provider or _active_provider
    use_model = model or _active_model

    # Build conversation history in portal format
    conv_history = []
    if system:
        conv_history.append({"role": "system", "content": system})
    if history:
        conv_history.extend(history)

    payload = {
        "provider": use_provider,
        "model": use_model,
        "message": prompt,
        "conversation_history": conv_history,
        "temperature": 0.7,
        "max_tokens": 4096,
    }

    for attempt in range(2):  # retry once after token refresh
        try:
            async with _session.post(
                f"{AI_PORTAL_URL}/chat/direct/stream",
                headers={
                    "Authorization": f"Bearer {AI_PORTAL_TOKEN}",
                    "Content-Type": "application/json",
                },
                json=payload,
                timeout=ClientTimeout(total=90),
            ) as resp:
                if resp.status == 401 and attempt == 0:
                    # Token expired — refresh and retry
                    if await _refresh_portal_token():
                        continue
                    return None

                if resp.status != 200:
                    body = await resp.text()
                    log.warning(f"Portal error {resp.status}: {body[:200]}")
                    return None

                # Parse SSE stream
                full_response = []
                async for line in resp.content:
                    line_str = line.decode("utf-8", errors="replace").strip()
                    if line_str.startswith("data: "):
                        try:
                            chunk = json.loads(line_str[6:])
                            content = chunk.get("content", "")
                            if content:
                                full_response.append(content)
                        except json.JSONDecodeError:
                            continue

                result = "".join(full_response).strip()
                if result:
                    return result
                return None
        except Exception as e:
            log.warning(f"Portal query failed ({use_provider}/{use_model}): {e}")
            return None

    return None


def resolve_model(name: str) -> Tuple[str, str, str]:
    """Resolve a model name/alias to (provider, model_id, display_name)."""
    name = name.strip().lower()
    # Check aliases first
    if name in MODEL_ALIASES:
        model_id = MODEL_ALIASES[name]
        info = PORTAL_MODELS.get(model_id, {})
        return info.get("provider", ""), model_id, info.get("name", model_id)
    # Check direct model IDs
    for mid, info in PORTAL_MODELS.items():
        if name == mid.lower() or name == info["name"].lower():
            return info["provider"], mid, info["name"]
    return "", "", ""


async def query_ai(prompt: str, system: Optional[str] = None,
                   channel: Optional[str] = None) -> str:
    """Query AI with fallback: Portal (active model) -> DeepSeek -> Groq -> xAI -> Ollama."""
    sys_prompt = system or BUNNY_ALPHA_PROMPT
    history = (await memory.get_history(channel)) if channel else []

    # Try AI Portal first (gives access to ALL models)
    if AI_PORTAL_TOKEN:
        result = await query_portal(prompt, sys_prompt, history)
        if result:
            model_info = PORTAL_MODELS.get(_active_model, {})
            name = model_info.get("name", _active_model)
            log.info(f"AI response from Portal/{name} ({len(result)} chars, {len(history)} history msgs)")
            return result

    # Fallback to direct API providers
    providers = [
        ("DeepSeek", query_deepseek),
        ("Groq", query_groq),
        ("xAI", query_xai),
        ("Ollama", query_ollama_chat),
    ]
    for name, fn in providers:
        result = await fn(prompt, sys_prompt, history)
        if result:
            log.info(f"AI response from {name} ({len(result)} chars, {len(history)} history msgs)")
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


async def post_message(text: str, channel: str, thread_ts: Optional[str] = None) -> Dict:
    """Post a message to Slack."""
    payload: Dict[str, Any] = {"channel": channel, "text": text}
    if thread_ts:
        payload["thread_ts"] = thread_ts
    result = await slack_post("chat.postMessage", payload)
    if not result.get("ok"):
        log.error(f"Slack post failed: {result.get('error')}")
    return result


async def add_reaction(channel: str, timestamp: str, emoji: str):
    """Add emoji reaction to a message."""
    await slack_post("reactions.add", {
        "channel": channel,
        "timestamp": timestamp,
        "name": emoji,
    })


async def update_message(text: str, channel: str, ts: str):
    """Update an existing message."""
    await slack_post("chat.update", {
        "channel": channel,
        "ts": ts,
        "text": text,
    })


async def post_image(image_url: str, alt_text: str, channel: str,
                     thread_ts: Optional[str] = None, title: str = ""):
    """Post an image to Slack using blocks."""
    blocks = [
        {
            "type": "image",
            "image_url": image_url,
            "alt_text": alt_text or "Generated image",
        }
    ]
    if title:
        blocks[0]["title"] = {"type": "plain_text", "text": title[:200]}

    payload: Dict[str, Any] = {
        "channel": channel,
        "text": alt_text,
        "blocks": blocks,
    }
    if thread_ts:
        payload["thread_ts"] = thread_ts
    result = await slack_post("chat.postMessage", payload)
    if not result.get("ok"):
        log.error(f"Slack image post failed: {result.get('error')}")
        # Fallback: post as plain URL
        await post_message(f":frame_with_picture: {image_url}", channel, thread_ts)
    return result


async def download_slack_file(file_url: str) -> Optional[bytes]:
    """Download a file from Slack (requires bot token auth)."""
    try:
        async with _session.get(
            file_url,
            headers={"Authorization": f"Bearer {SLACK_BOT_TOKEN}"},
            timeout=ClientTimeout(total=30),
        ) as resp:
            if resp.status == 200:
                return await resp.read()
            log.warning(f"Failed to download Slack file: {resp.status}")
            return None
    except Exception as e:
        log.warning(f"Slack file download failed: {e}")
        return None


async def describe_image_with_vision(image_url: str, user_text: str = "") -> Optional[str]:
    """Use xAI Grok vision to describe/analyze an image."""
    if not XAI_API_KEY:
        return None
    try:
        prompt = user_text or "Describe this image in detail."
        async with _session.post(
            "https://api.x.ai/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {XAI_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "grok-2-vision-latest",
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            {"type": "image_url", "image_url": {"url": image_url}},
                            {"type": "text", "text": prompt},
                        ],
                    }
                ],
                "max_tokens": 1000,
                "temperature": 0.7,
            },
            timeout=ClientTimeout(total=60),
        ) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"]["content"]
            log.warning(f"Vision API error: {data}")
            return None
    except Exception as e:
        log.warning(f"Vision API failed: {e}")
        return None


# ---------------------------------------------------------------------------
# Command Router & Parser
# ---------------------------------------------------------------------------

SLASH_COMMANDS = {
    "status": "Show system status across all VMs",
    "tasks": "Show current and recent tasks",
    "vms": "List all VMs with connectivity",
    "docker": "List Docker containers on swarm-mainframe",
    "gpu": "Show GPU status on swarm-gpu",
    "models": "List all available AI models (26+)",
    "model": "Switch active model (e.g. /model gpt5, /model claude)",
    "logs": "Show recent Bunny Alpha logs",
    "health": "Run health check on all services",
    "memory": "Show persistent memory stats (/memory search <query>)",
    "forget": "Clear memory (/forget, /forget all, /forget thread, /forget channel)",
    "pref": "Set/get preferences (/pref key value, /pref key, /pref)",
    "help": "Show available commands",
}


async def handle_slash_command(cmd: str, args: str, channel: str, thread_ts: str) -> bool:
    """Handle built-in slash commands. Returns True if handled."""
    global _active_provider, _active_model
    cmd = cmd.lower().strip()

    if cmd == "help":
        lines = [":bunny: *Bunny Alpha Commands*\n"]
        for c, desc in SLASH_COMMANDS.items():
            lines.append(f"\u2022 `/{c}` \u2014 {desc}")
        lines.append("\nOr just tell me what you need in plain English!")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "memory":
        sub = args.strip().lower()
        if sub == "stats" or not sub:
            s = await memory.stats()
            ch_count = len(s["channels"])
            total = s["total_messages"]
            this_ch = s["channels"].get(channel, 0)
            db_kb = s["db_size_bytes"] / 1024
            lines = [":brain: *Persistent Memory*\n"]
            lines.append(f"*This channel:* {this_ch} messages")
            lines.append(f"*All channels:* {total} messages across {ch_count} channels")
            lines.append(f"*Summaries:* {s['summaries']}")
            lines.append(f"*Task runs:* {s['task_runs']}")
            lines.append(f"*Preferences:* {s['preferences']}")
            lines.append(f"*DB size:* {db_kb:.1f} KB")
            if s["oldest_message"]:
                import datetime
                age = datetime.datetime.fromtimestamp(s["oldest_message"]).strftime("%Y-%m-%d %H:%M")
                lines.append(f"*Oldest message:* {age}")
            lines.append(f"\n_Context window: {MEMORY_SIZE} messages | Auto-summarize at {SUMMARIZE_THRESHOLD}_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif sub == "search" and len(args.split()) > 1:
            query = " ".join(args.split()[1:])
            results = await memory.search_messages(query, limit=10)
            if results:
                lines = [f":mag: *Memory search:* `{query}` ({len(results)} results)\n"]
                for r in results:
                    import datetime
                    ts = datetime.datetime.fromtimestamp(r["created_at"]).strftime("%m/%d %H:%M")
                    snippet = r["content"][:120].replace("\n", " ")
                    lines.append(f"`{ts}` [{r['role']}] {snippet}")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(f":mag: No results for `{query}`", channel, thread_ts)
        return True

    if cmd == "forget":
        sub = args.strip().lower()
        if sub == "all":
            await memory.clear_all()
            await post_message(":wastebasket: All conversation memory cleared.", channel, thread_ts)
        elif sub == "thread" and thread_ts:
            await memory.clear(channel, thread_ts)
            await post_message(":wastebasket: Memory cleared for this thread.", channel, thread_ts)
        elif sub.startswith("channel"):
            target_ch = sub.split()[-1] if len(sub.split()) > 1 else channel
            await memory.clear(target_ch)
            await post_message(f":wastebasket: Memory cleared for channel `{target_ch}`.", channel, thread_ts)
        else:
            await memory.clear(channel)
            await post_message(":wastebasket: Memory cleared for this channel.", channel, thread_ts)
        return True

    if cmd == "pref":
        parts = args.strip().split(maxsplit=1)
        if len(parts) == 2:
            key, value = parts
            await memory.set_preference("global", key, value)
            await post_message(f":gear: Preference set: `{key}` = `{value}`", channel, thread_ts)
        elif len(parts) == 1:
            val = await memory.get_preference("global", parts[0])
            if val:
                await post_message(f":gear: `{parts[0]}` = `{val}`", channel, thread_ts)
            else:
                await post_message(f":gear: Preference `{parts[0]}` not set.", channel, thread_ts)
        else:
            prefs = await memory.get_all_preferences("global")
            if prefs:
                lines = [":gear: *Preferences*\n"]
                for k, v in prefs.items():
                    lines.append(f"  `{k}` = `{v}`")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":gear: No preferences set.", channel, thread_ts)
        return True

    if cmd == "status":
        group_id = uuid.uuid4().hex[:8]
        commands = [
            ("shell", "swarm-mainframe", "uptime && free -h | head -2"),
            ("shell", "swarm-mainframe", "docker ps --format 'table {{.Names}}\\t{{.Status}}\\t{{.Ports}}'"),
            ("shell", "swarm-gpu", "uptime && nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total --format=csv,noheader 2>/dev/null || echo 'GPU unavailable'"),
            ("shell", "swarm-gpu", "free -h | head -2"),
        ]
        for tool, host, c in commands:
            task_manager.create_task(tool, host, c, channel, thread_ts, group_id)
        tasks = await task_manager.execute_group(group_id, channel, thread_ts)
        await _post_task_results(tasks, channel, thread_ts, "System Status")
        return True

    if cmd == "tasks":
        recent = task_manager.get_recent_tasks(10)
        if not recent:
            await post_message(":clipboard: No tasks yet.", channel, thread_ts)
            return True
        lines = [":clipboard: *Recent Tasks*\n"]
        for t in recent:
            icon = {
                TaskStatus.COMPLETED: ":white_check_mark:",
                TaskStatus.FAILED: ":x:",
                TaskStatus.RUNNING: ":hourglass_flowing_sand:",
                TaskStatus.QUEUED: ":inbox_tray:",
                TaskStatus.CANCELLED: ":no_entry_sign:",
            }.get(t.status, ":grey_question:")
            dur = f" ({t.duration}s)" if t.duration else ""
            lines.append(f"{icon} `{t.short_id}` {t.tool}@{t.host}: `{t.cmd[:40]}`{dur}")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "vms":
        group_id = uuid.uuid4().hex[:8]
        for vm_name in VMS:
            task_manager.create_task("shell", vm_name, "uptime 2>/dev/null || echo 'unreachable'",
                                     channel, thread_ts, group_id)
        tasks = await task_manager.execute_group(group_id, channel, thread_ts)
        await _post_task_results(tasks, channel, thread_ts, "VM Status")
        return True

    if cmd == "docker":
        host = args.strip() or "swarm-mainframe"
        group_id = uuid.uuid4().hex[:8]
        task_manager.create_task("shell", host,
                                 "docker ps -a --format 'table {{.Names}}\\t{{.Status}}\\t{{.Image}}'",
                                 channel, thread_ts, group_id)
        tasks = await task_manager.execute_group(group_id, channel, thread_ts)
        await _post_task_results(tasks, channel, thread_ts, f"Docker on {host}")
        return True

    if cmd == "gpu":
        group_id = uuid.uuid4().hex[:8]
        task_manager.create_task("shell", "swarm-gpu", "nvidia-smi", channel, thread_ts, group_id)
        tasks = await task_manager.execute_group(group_id, channel, thread_ts)
        await _post_task_results(tasks, channel, thread_ts, "GPU Status")
        return True

    if cmd == "models":
        # Show all AI Portal models grouped by provider
        current = PORTAL_MODELS.get(_active_model, {})
        current_name = current.get("name", _active_model)
        lines = [f":brain: *Available AI Models* (active: *{current_name}*)\n"]
        by_provider: Dict[str, List[str]] = {}
        for mid, info in PORTAL_MODELS.items():
            p = info["provider"]
            if p not in by_provider:
                by_provider[p] = []
            marker = " :star:" if mid == _active_model else ""
            by_provider[p].append(f"`{mid}` \u2014 {info['name']}{marker}")
        for p, models in by_provider.items():
            lines.append(f"*{p.upper()}*")
            for m in models:
                lines.append(f"  \u2022 {m}")
        lines.append(f"\n_Switch with_ `/model <name>` _or aliases:_ `gpt5`, `claude`, `gemini`, `grok`, `r1`, `llama`")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "model":
        if not args.strip():
            current = PORTAL_MODELS.get(_active_model, {})
            await post_message(
                f":gear: Active model: *{current.get('name', _active_model)}* (`{_active_model}` via `{_active_provider}`)",
                channel, thread_ts,
            )
            return True
        provider, model_id, display = resolve_model(args.strip())
        if not model_id:
            await post_message(
                f":warning: Unknown model `{args.strip()}`. Try `/models` to see available options.",
                channel, thread_ts,
            )
            return True
        _active_provider = provider
        _active_model = model_id
        await post_message(
            f":white_check_mark: Switched to *{display}* (`{model_id}` via `{provider}`)",
            channel, thread_ts,
        )
        return True

    if cmd == "logs":
        count = args.strip() or "20"
        group_id = uuid.uuid4().hex[:8]
        task_manager.create_task("shell", "swarm-mainframe",
                                 f"journalctl -u bunny-alpha --no-pager -n {count} --output=short",
                                 channel, thread_ts, group_id)
        tasks = await task_manager.execute_group(group_id, channel, thread_ts)
        await _post_task_results(tasks, channel, thread_ts, "Bunny Alpha Logs")
        return True

    if cmd == "health":
        group_id = uuid.uuid4().hex[:8]
        commands = [
            ("shell", "swarm-mainframe", "systemctl is-active bunny-alpha docker"),
            ("shell", "swarm-mainframe", "docker ps --filter 'status=running' --format '{{.Names}}: {{.Status}}'"),
            ("http", "local", "GET http://localhost:8090/health"),
            ("shell", "swarm-gpu", "systemctl is-active ollama 2>/dev/null || curl -s http://localhost:11434/api/tags > /dev/null && echo 'ollama: active' || echo 'ollama: inactive'"),
            ("shell", "swarm-mainframe", "curl -s http://localhost:8080/health 2>/dev/null || echo 'SWARM: not responding'"),
        ]
        for tool, host, c in commands:
            task_manager.create_task(tool, host, c, channel, thread_ts, group_id)
        tasks = await task_manager.execute_group(group_id, channel, thread_ts)
        await _post_task_results(tasks, channel, thread_ts, "Health Check")
        return True

    return False


def parse_execute_blocks(text: str) -> List[Dict[str, str]]:
    """Parse [EXECUTE]...[/EXECUTE] blocks from AI response."""
    commands = []
    pattern = r'\[EXECUTE\](.*?)\[/EXECUTE\]'
    matches = re.findall(pattern, text, re.DOTALL)

    for block in matches:
        for line in block.strip().split("\n"):
            line = line.strip()
            if not line:
                continue
            try:
                cmd_data = json.loads(line)
                commands.append({
                    "tool": cmd_data.get("tool", "shell"),
                    "host": cmd_data.get("host", "swarm-mainframe"),
                    "cmd": cmd_data.get("cmd", cmd_data.get("command", "")),
                })
            except json.JSONDecodeError:
                log.warning(f"Failed to parse command: {line}")
                continue

    return commands


def extract_chat_text(text: str) -> str:
    """Remove [EXECUTE] blocks and return the chat portion."""
    cleaned = re.sub(r'\[EXECUTE\].*?\[/EXECUTE\]', '', text, flags=re.DOTALL)
    return cleaned.strip()


async def _post_task_results(tasks: List[Task], channel: str, thread_ts: str, title: str = "Results"):
    """Post formatted task results to Slack."""
    lines = [f":white_check_mark: *{title}*\n"]

    for t in tasks:
        icon = ":white_check_mark:" if t.status == TaskStatus.COMPLETED else ":x:"
        dur = f" _({t.duration}s)_" if t.duration else ""
        header = f"{icon} *{t.host}*: `{t.cmd[:50]}`{dur}"
        lines.append(header)

        output = t.result if t.status == TaskStatus.COMPLETED else (t.error or "Unknown error")
        if output:
            # Truncate long outputs
            if len(output) > 800:
                output = output[:800] + "\n...(truncated)"
            lines.append(f"```{output}```")

    full_text = "\n".join(lines)
    # Slack message limit
    if len(full_text) > 3900:
        full_text = full_text[:3900] + "\n...(message truncated)"

    await post_message(full_text, channel, thread_ts)


# ---------------------------------------------------------------------------
# Request Verification
# ---------------------------------------------------------------------------

def verify_slack_signature(body: bytes, timestamp: str, signature: str) -> bool:
    """Verify request is from Slack using signing secret."""
    if not SLACK_SIGNING_SECRET:
        return True
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
        "Event received: type=%s subtype=%s user=%s text=%s",
        event.get("type"), event.get("subtype"),
        event.get("user"), str(event.get("text", ""))[:50]
    )

    # Dedup
    now = time.time()
    if event_id in _seen_events:
        return web.Response(status=200, text="ok")
    _seen_events[event_id] = now
    for k in [k for k, v in _seen_events.items() if now - v > 300]:
        del _seen_events[k]

    if (
        event.get("type") in ("app_mention", "message")
        and not event.get("bot_id")
        and not event.get("subtype")
    ):
        text = event.get("text", "").strip()
        if BOT_USER_ID:
            text = text.replace(f"<@{BOT_USER_ID}>", "").strip()

        channel = event["channel"]
        thread_ts = event.get("thread_ts") or event.get("ts")

        # Check for image files attached to message
        files = event.get("files", [])
        image_files = [
            f for f in files
            if f.get("mimetype", "").startswith("image/")
        ]

        if image_files:
            # User sent an image — use vision to analyze it
            asyncio.create_task(
                _process_image(image_files, text, channel, thread_ts)
            )
        elif text:
            asyncio.create_task(_process_message(text, channel, thread_ts))

    return web.Response(status=200, text="ok")


async def _process_message(text: str, channel: str, thread_ts: Optional[str]):
    """Process a message — route to commands or AI, with conversation memory."""
    try:
        log.info(f"Processing: {text[:80]}...")

        # Check for slash commands
        if text.startswith("/"):
            parts = text[1:].split(None, 1)
            cmd = parts[0] if parts else ""
            args = parts[1] if len(parts) > 1 else ""
            if await handle_slash_command(cmd, args, channel, thread_ts):
                return

        # Store user message in persistent memory
        await memory.add(channel, "user", text)

        # Auto-summarize old messages if threshold exceeded
        summarize_data = await memory.auto_summarize_if_needed(channel)
        if summarize_data:
            try:
                summary_text = await query_ai(
                    f"Summarize this conversation history in 2-3 sentences for future context:\n\n{summarize_data['text'][:2000]}",
                    system="You are a helpful summarizer. Produce a concise summary of the key topics and outcomes.",
                )
                if summary_text:
                    await memory.complete_summarize(channel, summary_text, summarize_data["ids"])
            except Exception as e:
                log.warning(f"Auto-summarize failed: {e}")

        # Send to AI with conversation history
        response = await query_ai(text, channel=channel)

        # Check if AI wants to execute commands
        commands = parse_execute_blocks(response)
        chat_text = extract_chat_text(response)

        if commands:
            # Post the chat portion first (if any)
            if chat_text:
                await post_message(chat_text, channel, thread_ts)

            # Execute all commands
            group_id = uuid.uuid4().hex[:8]
            for cmd_data in commands:
                task_manager.create_task(
                    cmd_data["tool"],
                    cmd_data["host"],
                    cmd_data["cmd"],
                    channel, thread_ts, group_id,
                )
            tasks = await task_manager.execute_group(group_id, channel, thread_ts)

            # Handle results — check for images vs text
            image_tasks = [t for t in tasks if t.result and t.result.startswith("IMAGE_URL:")]
            text_tasks = [t for t in tasks if t not in image_tasks]

            # Post generated images directly
            for t in image_tasks:
                img_url = t.result.replace("IMAGE_URL:", "").strip()
                await post_image(img_url, t.cmd[:100], channel, thread_ts, title=t.cmd[:100])
                await memory.add(channel, "assistant", f"[Generated image: {t.cmd[:80]}]")

            # Summarize text results if any
            if text_tasks:
                result_text = ""
                for t in text_tasks:
                    result_text += f"\n--- {t.tool}@{t.host}: {t.cmd} ---\n"
                    if t.status == TaskStatus.COMPLETED:
                        result_text += t.result or "(no output)"
                    else:
                        result_text += f"FAILED: {t.error}"
                    result_text += "\n"

                summary_prompt = (
                    f"You ran these commands for Sean. Here are the results. "
                    f"Give a concise, friendly summary. Use Slack formatting.\n\n"
                    f"Original request: {text}\n\nResults:{result_text}"
                )
                summary = await query_ai(summary_prompt, channel=channel)
                summary = extract_chat_text(summary)
                if summary:
                    await post_message(summary, channel, thread_ts)
                    await memory.add(channel, "assistant", summary)
            elif not image_tasks:
                await memory.add(channel, "assistant", f"[Executed {len(tasks)} tasks]")
        else:
            # Pure chat response
            if len(response) > 3900:
                response = response[:3900] + "\n...(truncated)"
            await post_message(response, channel, thread_ts)
            # Store assistant response in memory
            await memory.add(channel, "assistant", response)

    except Exception as e:
        log.error(f"Message processing failed: {e}", exc_info=True)
        await post_message(
            f":x: Bunny Alpha error: `{e}`",
            channel, thread_ts,
        )


async def _process_image(files: List[Dict], text: str, channel: str, thread_ts: Optional[str]):
    """Process an image shared in Slack using vision API."""
    try:
        for f in files[:3]:  # Max 3 images per message
            file_url = f.get("url_private", "")
            filename = f.get("name", "image")
            log.info(f"Processing image: {filename}")

            if not file_url:
                await post_message(":warning: Couldn't access image file.", channel, thread_ts)
                continue

            # Try direct URL with Slack token for vision API
            # Download the image data first
            image_data = await download_slack_file(file_url)
            if not image_data:
                await post_message(f":warning: Couldn't download `{filename}`.", channel, thread_ts)
                continue

            # For vision, we need a publicly accessible URL or base64
            # Use base64 data URL
            import base64
            mimetype = f.get("mimetype", "image/png")
            b64 = base64.b64encode(image_data).decode("utf-8")
            data_url = f"data:{mimetype};base64,{b64}"

            prompt = text if text else "Describe this image in detail. What do you see?"
            await memory.add(channel, "user", f"[Shared image: {filename}] {prompt}")

            description = await describe_image_with_vision(data_url, prompt)
            if description:
                await post_message(description, channel, thread_ts)
                await memory.add(channel, "assistant", description)
            else:
                await post_message(
                    ":eyes: I can see you shared an image, but my vision API isn't available right now. "
                    "Try again in a moment!",
                    channel, thread_ts,
                )
    except Exception as e:
        log.error(f"Image processing failed: {e}", exc_info=True)
        await post_message(f":x: Image processing error: `{e}`", channel, thread_ts)


# ---------------------------------------------------------------------------
# API Endpoints
# ---------------------------------------------------------------------------

async def handle_health(request: web.Request) -> web.Response:
    """Health check endpoint."""
    active = task_manager.get_active_tasks()
    return web.json_response({
        "status": "healthy",
        "service": "bunny-alpha",
        "version": "2.0.0",
        "active_tasks": len(active),
        "total_tasks": len(task_manager.tasks),
        "providers": {
            "deepseek": bool(DEEPSEEK_API_KEY),
            "groq": bool(GROQ_API_KEY),
            "xai": bool(XAI_API_KEY),
            "ollama": bool(OLLAMA_URL),
        },
    })


async def handle_tasks_api(request: web.Request) -> web.Response:
    """API endpoint to view tasks."""
    recent = task_manager.get_recent_tasks(20)
    return web.json_response({
        "tasks": [
            {
                "id": t.task_id,
                "tool": t.tool,
                "host": t.host,
                "cmd": t.cmd,
                "status": t.status.value,
                "result": (t.result or "")[:500],
                "error": t.error,
                "duration": t.duration,
                "created_at": t.created_at,
            }
            for t in recent
        ]
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
            f"Bunny Alpha v2.0 online | bot={result['user']} | "
            f"team={result['team']} | user_id={BOT_USER_ID}"
        )
    else:
        log.error(f"Slack auth failed: {result.get('error')}")

    # AI Portal status
    if AI_PORTAL_TOKEN:
        try:
            async with _session.get(
                f"{AI_PORTAL_URL}/chat/direct/models",
                headers={"Authorization": f"Bearer {AI_PORTAL_TOKEN}"},
                timeout=ClientTimeout(total=5),
            ) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    model_count = len(data) if isinstance(data, list) else "?"
                    log.info(f"AI Portal connected: {AI_PORTAL_URL} ({model_count} models available)")
                    log.info(f"Active model: {_active_model} via {_active_provider}")
                else:
                    log.warning(f"AI Portal responded {resp.status} — may need token refresh")
        except Exception as e:
            log.warning(f"AI Portal unreachable: {e}")
    else:
        log.info("AI Portal: not configured (no token)")

    # Direct API providers (fallback chain)
    providers = []
    if DEEPSEEK_API_KEY:
        providers.append("DeepSeek")
    if GROQ_API_KEY:
        providers.append("Groq")
    if XAI_API_KEY:
        providers.append("xAI")
    if OLLAMA_URL:
        providers.append(f"Ollama({OLLAMA_URL})")

    log.info(f"Fallback providers: {', '.join(providers) or 'NONE'}")
    log.info(f"VMs: {', '.join(VMS.keys())}")
    log.info(f"Max concurrent tasks: {MAX_CONCURRENT_TASKS}")

    # Memory stats
    try:
        mem_stats = await memory.stats()
        log.info(
            f"Persistent memory: {mem_stats['total_messages']} messages, "
            f"{mem_stats['summaries']} summaries, "
            f"{mem_stats['task_runs']} task runs, "
            f"DB={mem_stats['db_size_bytes']/1024:.1f}KB"
        )
    except Exception as e:
        log.warning(f"Memory stats unavailable: {e}")

    log.info(f"Listening on port {PORT}")

    # Start periodic cleanup
    asyncio.create_task(_periodic_cleanup())


async def _periodic_cleanup():
    """Clean up old tasks periodically."""
    while True:
        await asyncio.sleep(300)
        task_manager.cleanup_old(3600)


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
    app.router.add_get("/tasks", handle_tasks_api)

    log.info("Starting Bunny Alpha v2.0 \u2014 Multi-task Infrastructure Operator")
    web.run_app(app, host="0.0.0.0", port=PORT)


if __name__ == "__main__":
    main()
