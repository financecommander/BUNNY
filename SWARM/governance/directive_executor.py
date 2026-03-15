"""
Directive Executor
==================
Takes a stored directive, calls Claude API to generate code,
writes files to the target repo, then triggers autopush.

Pipeline:
  directive_store (acknowledged)
      → generate_code()  — Claude API
      → autopush.push_inline_code()
      → directive_store (pushed)
      → Signal confirmation to sender

Environment variables:
  ANTHROPIC_API_KEY   — required for code generation
  SIGNAL_POLL_INTERVAL_S — ingress polling interval (default 10s)
"""

import json
import os
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

import anthropic

from .directive_store import DirectiveRecord, list_all, pending, update_status
from .autopush import push_inline_code, repo_for_directive
from ..comms.portal_client import get_anthropic_key

MODEL = os.getenv("DIRECTIVE_MODEL", "claude-opus-4-6")

# Max tokens for code generation
MAX_TOKENS = int(os.getenv("DIRECTIVE_MAX_TOKENS", "8192"))

_SYSTEM_PROMPT = """\
You are BUNNY, the autonomous infrastructure AI for the Calculus Holdings swarm platform.

You receive directives from the owner (Sean) via Signal. Your job is to translate each directive into production-ready Python code and output it as a JSON object.

Rules:
- Generate complete, working, production-quality code — no stubs, no TODOs
- Follow existing patterns in the codebase (FastAPI, asyncio, dataclasses, type hints)
- Each file must be self-contained or use only existing swarm imports
- Output ONLY a JSON object with this exact structure:

{
  "summary": "one-line description of what was built",
  "files": {
    "relative/path/to/file.py": "...complete file contents...",
    "another/file.py": "...complete file contents..."
  },
  "deploy_notes": "any special deploy steps needed (optional)"
}

Do not output anything outside the JSON object.
"""


def _call_claude(directive_text: str, repo_context: str = "") -> dict:
    """Call Claude API via key fetched from AI Portal."""
    api_key = get_anthropic_key()
    client = anthropic.Anthropic(api_key=api_key)

    user_content = f"DIRECTIVE:\n\n{directive_text}"
    if repo_context:
        user_content += f"\n\nREPO CONTEXT:\n{repo_context}"

    message = client.messages.create(
        model=MODEL,
        max_tokens=MAX_TOKENS,
        system=_SYSTEM_PROMPT,
        messages=[{"role": "user", "content": user_content}],
    )

    raw = message.content[0].text.strip()

    # Strip markdown code fences if present
    if raw.startswith("```"):
        raw = re.sub(r"^```[a-z]*\n?", "", raw)
        raw = re.sub(r"\n?```$", "", raw)

    return json.loads(raw)


def _repo_context(record: DirectiveRecord) -> str:
    """Build a brief repo context string to help Claude generate consistent code."""
    from .autopush import repo_for_directive
    repo = repo_for_directive(record.directive_id)
    if not repo.exists():
        return ""
    lines = [f"Target repo: {repo.name}"]
    # List top-level dirs
    dirs = [p.name for p in repo.iterdir() if p.is_dir() and not p.name.startswith(".")][:12]
    if dirs:
        lines.append(f"Top-level dirs: {', '.join(sorted(dirs))}")
    return "\n".join(lines)


def execute(record: DirectiveRecord) -> dict:
    """
    Execute a single directive: generate code + push.

    Returns dict with keys: success, commit_hash, error, files_written
    """
    result = {"success": False, "commit_hash": "", "error": "", "files_written": []}

    api_key = get_anthropic_key()
    if not api_key:
        result["error"] = "Anthropic key unavailable — check AI Portal config"
        update_status(record.directive_id, "error", error=result["error"])
        return result

    print(f"[executor] generating code for {record.directive_id}...", flush=True)
    update_status(record.directive_id, "building")

    try:
        context = _repo_context(record)
        generated = _call_claude(record.raw_text, context)
    except json.JSONDecodeError as e:
        result["error"] = f"Claude response parse error: {e}"
        update_status(record.directive_id, "error", error=result["error"])
        return result
    except Exception as e:
        result["error"] = f"Claude API error: {e}"
        update_status(record.directive_id, "error", error=result["error"])
        return result

    files: dict[str, str] = generated.get("files", {})
    summary: str = generated.get("summary", record.summary)
    deploy_notes: str = generated.get("deploy_notes", "")

    if not files:
        result["error"] = "Claude returned no files"
        update_status(record.directive_id, "error", error=result["error"])
        return result

    print(f"[executor] {len(files)} file(s) generated — pushing...", flush=True)
    if deploy_notes:
        print(f"[executor] deploy notes: {deploy_notes}", flush=True)

    push_result = push_inline_code(
        directive_id=record.directive_id,
        code_blocks=files,
        summary=summary,
        notify_number=record.source_number,
    )

    result["success"] = push_result["success"]
    result["commit_hash"] = push_result.get("commit_hash", "")
    result["error"] = push_result.get("error", "")
    result["files_written"] = list(files.keys())

    if result["success"]:
        print(
            f"[executor] pushed {record.directive_id} → {result['commit_hash'][:8]}  "
            f"({len(files)} files)",
            flush=True,
        )
    else:
        print(f"[executor] push failed: {result['error']}", flush=True)

    return result


def execute_pending() -> list[dict]:
    """Execute all pending (acknowledged) directives. Called by ingress daemon."""
    results = []
    for record in pending():
        r = execute(record)
        r["directive_id"] = record.directive_id
        results.append(r)
    return results


def status_report() -> str:
    """Human-readable status of all directives."""
    records = list_all()
    if not records:
        return "No directives on record."
    lines = [
        f"{'#':<3}  {'directive_id':<30}  {'status':<12}  {'commit':<9}  summary",
        "-" * 95,
    ]
    for i, r in enumerate(records, 1):
        commit = r.commit_hash[:8] if r.commit_hash else "—"
        lines.append(
            f"{i:<3}  {r.directive_id:<30}  {r.status:<12}  {commit:<9}  {r.summary[:45]}"
        )
    return "\n".join(lines)
