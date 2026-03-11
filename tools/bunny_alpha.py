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
import hashlib
import hmac
import json
import logging
import os
import re
import time
import uuid
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Optional, Tuple

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
PORT = int(os.environ.get("BUNNY_ALPHA_PORT", "8090"))
BOT_USER_ID: str = ""

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


# Singleton instances
task_manager = TaskManager()
tool_executor = ToolExecutor()


# ---------------------------------------------------------------------------
# AI Model Providers
# ---------------------------------------------------------------------------

async def query_deepseek(prompt: str, system: str) -> Optional[str]:
    """Query DeepSeek API directly."""
    if not DEEPSEEK_API_KEY:
        return None
    try:
        async with _session.post(
            "https://api.deepseek.com/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {DEEPSEEK_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "deepseek-chat",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
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


async def query_groq(prompt: str, system: str) -> Optional[str]:
    """Query Groq API directly."""
    if not GROQ_API_KEY:
        return None
    try:
        async with _session.post(
            "https://api.groq.com/openai/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {GROQ_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "meta-llama/llama-4-scout-17b-16e-instruct",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
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


async def query_xai(prompt: str, system: str) -> Optional[str]:
    """Query xAI/Grok API directly."""
    if not XAI_API_KEY:
        return None
    try:
        async with _session.post(
            "https://api.x.ai/v1/chat/completions",
            headers={
                "Authorization": f"Bearer {XAI_API_KEY}",
                "Content-Type": "application/json",
            },
            json={
                "model": "grok-3-fast",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
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


async def query_ollama_chat(prompt: str, system: str) -> Optional[str]:
    """Query local Ollama instance for chat."""
    if not OLLAMA_URL:
        return None
    try:
        async with _session.post(
            f"{OLLAMA_URL}/api/chat",
            json={
                "model": "qwen2.5-coder:7b",
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": prompt},
                ],
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


async def query_ai(prompt: str, system: Optional[str] = None) -> str:
    """Query AI with fallback chain: DeepSeek -> Groq -> xAI -> Ollama."""
    sys_prompt = system or BUNNY_ALPHA_PROMPT
    providers = [
        ("DeepSeek", query_deepseek),
        ("Groq", query_groq),
        ("xAI", query_xai),
        ("Ollama", query_ollama_chat),
    ]
    for name, fn in providers:
        result = await fn(prompt, sys_prompt)
        if result:
            log.info(f"AI response from {name} ({len(result)} chars)")
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


# ---------------------------------------------------------------------------
# Command Router & Parser
# ---------------------------------------------------------------------------

SLASH_COMMANDS = {
    "status": "Show system status across all VMs",
    "tasks": "Show current and recent tasks",
    "vms": "List all VMs with connectivity",
    "docker": "List Docker containers on swarm-mainframe",
    "gpu": "Show GPU status on swarm-gpu",
    "models": "List available Ollama models",
    "logs": "Show recent Bunny Alpha logs",
    "health": "Run health check on all services",
    "help": "Show available commands",
}


async def handle_slash_command(cmd: str, args: str, channel: str, thread_ts: str) -> bool:
    """Handle built-in slash commands. Returns True if handled."""
    cmd = cmd.lower().strip()

    if cmd == "help":
        lines = [":bunny: *Bunny Alpha Commands*\n"]
        for c, desc in SLASH_COMMANDS.items():
            lines.append(f"\u2022 `/{c}` \u2014 {desc}")
        lines.append("\nOr just tell me what you need in plain English!")
        await post_message("\n".join(lines), channel, thread_ts)
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
        group_id = uuid.uuid4().hex[:8]
        task_manager.create_task("shell", "swarm-gpu",
                                 "curl -s http://localhost:11434/api/tags | python3 -m json.tool 2>/dev/null || echo 'Ollama unreachable'",
                                 channel, thread_ts, group_id)
        tasks = await task_manager.execute_group(group_id, channel, thread_ts)
        await _post_task_results(tasks, channel, thread_ts, "Ollama Models")
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

        if text:
            channel = event["channel"]
            thread_ts = event.get("thread_ts") or event.get("ts")
            asyncio.create_task(_process_message(text, channel, thread_ts))

    return web.Response(status=200, text="ok")


async def _process_message(text: str, channel: str, thread_ts: Optional[str]):
    """Process a message — route to commands or AI."""
    try:
        log.info(f"Processing: {text[:80]}...")

        # Check for slash commands
        if text.startswith("/"):
            parts = text[1:].split(None, 1)
            cmd = parts[0] if parts else ""
            args = parts[1] if len(parts) > 1 else ""
            if await handle_slash_command(cmd, args, channel, thread_ts):
                return

        # Send to AI
        response = await query_ai(text)

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

            # Collect results and send to AI for summary
            result_text = ""
            for t in tasks:
                result_text += f"\n--- {t.tool}@{t.host}: {t.cmd} ---\n"
                if t.status == TaskStatus.COMPLETED:
                    result_text += t.result or "(no output)"
                else:
                    result_text += f"FAILED: {t.error}"
                result_text += "\n"

            # Get AI summary of results
            summary_prompt = (
                f"You ran these commands for Sean. Here are the results. "
                f"Give a concise, friendly summary. Use Slack formatting.\n\n"
                f"Original request: {text}\n\nResults:{result_text}"
            )
            summary = await query_ai(summary_prompt)

            # Remove any execute blocks from summary
            summary = extract_chat_text(summary)
            if summary:
                await post_message(summary, channel, thread_ts)
        else:
            # Pure chat response
            if len(response) > 3900:
                response = response[:3900] + "\n...(truncated)"
            await post_message(response, channel, thread_ts)

    except Exception as e:
        log.error(f"Message processing failed: {e}", exc_info=True)
        await post_message(
            f":x: Bunny Alpha error: `{e}`",
            channel, thread_ts,
        )


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

    providers = []
    if DEEPSEEK_API_KEY:
        providers.append("DeepSeek")
    if GROQ_API_KEY:
        providers.append("Groq")
    if XAI_API_KEY:
        providers.append("xAI")
    if OLLAMA_URL:
        providers.append(f"Ollama({OLLAMA_URL})")

    log.info(f"AI providers: {', '.join(providers) or 'NONE'}")
    log.info(f"VMs: {', '.join(VMS.keys())}")
    log.info(f"Max concurrent tasks: {MAX_CONCURRENT_TASKS}")
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
