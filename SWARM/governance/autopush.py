"""
Directive Auto-Push
===================
Automatically commits and pushes code generated from Signal directives.

When a directive produces files, call push_directive_code() to:
  1. Stage the files
  2. Commit with a standardised message
  3. Push to origin/main
  4. Update directive_store with commit hash + status
  5. Send a Signal confirmation back to the sender

Repo routing (directive prefix → local repo path):
  SWARM-*   → SWARM_REPO  (super-duper-spork)
  BUNNY-*   → BUNNY_REPO
  TRITON-*  → TRITON_REPO
  default   → SWARM_REPO
"""

import os
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from .directive_store import update_status

# ---------------------------------------------------------------------------
# Repo paths — override via env on the server
# ---------------------------------------------------------------------------
SWARM_REPO  = Path(os.getenv("SWARM_REPO_PATH",  "/opt/repos/super-duper-spork"))
BUNNY_REPO  = Path(os.getenv("BUNNY_REPO_PATH",  "/opt/repos/BUNNY"))
TRITON_REPO = Path(os.getenv("TRITON_REPO_PATH", "/opt/repos/Triton"))

_REPO_MAP: dict[str, Path] = {
    "SWARM":  SWARM_REPO,
    "BUNNY":  BUNNY_REPO,
    "TRITON": TRITON_REPO,
}

GIT_AUTHOR_NAME  = os.getenv("GIT_AUTHOR_NAME",  "Swarm Mainframe")
GIT_AUTHOR_EMAIL = os.getenv("GIT_AUTHOR_EMAIL", "swarm@calculusholdings.com")


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def repo_for_directive(directive_id: str) -> Path:
    """Return the repo path for a directive based on its prefix."""
    prefix = directive_id.split("-")[0].upper()
    return _REPO_MAP.get(prefix, SWARM_REPO)


def push_directive_code(
    directive_id: str,
    files: list[Path],
    summary: str,
    *,
    repo_path: Optional[Path] = None,
    branch: str = "main",
    notify_number: Optional[str] = None,
) -> dict:
    """
    Stage, commit, and push files for a directive.

    Args:
        directive_id:   e.g. "SWARM-L4-UPGRADE-01"
        files:          List of absolute paths to add (must be inside repo_path)
        summary:        One-line description for the commit message
        repo_path:      Override repo (default: derived from directive_id prefix)
        branch:         Target branch (default: main)
        notify_number:  Signal number to notify on success/failure

    Returns:
        dict with keys: success, commit_hash, error
    """
    repo = repo_path or repo_for_directive(directive_id)

    result = {"success": False, "commit_hash": "", "error": ""}

    if not repo.exists():
        result["error"] = f"Repo not found: {repo}"
        update_status(directive_id, "error", error=result["error"])
        _notify(notify_number, directive_id, success=False, detail=result["error"])
        return result

    env = {
        **os.environ,
        "GIT_AUTHOR_NAME": GIT_AUTHOR_NAME,
        "GIT_AUTHOR_EMAIL": GIT_AUTHOR_EMAIL,
        "GIT_COMMITTER_NAME": GIT_AUTHOR_NAME,
        "GIT_COMMITTER_EMAIL": GIT_AUTHOR_EMAIL,
    }

    try:
        # Stage files
        for f in files:
            _git(["add", str(f)], cwd=repo, env=env)

        # Check if there's anything to commit
        status = _git(["status", "--porcelain"], cwd=repo, env=env)
        if not status.strip():
            result["error"] = "No changes to commit"
            update_status(directive_id, "error", error=result["error"])
            return result

        # Commit
        msg = f"directive: {directive_id} — {summary}\n\nAuto-generated from Signal directive.\nCo-Authored-By: BUNNY <bunny@calculusholdings.com>"
        _git(["commit", "-m", msg], cwd=repo, env=env)

        # Get commit hash
        commit_hash = _git(["rev-parse", "HEAD"], cwd=repo, env=env).strip()

        # Push
        _git(["push", "origin", branch], cwd=repo, env=env)

        pushed_at = datetime.now(timezone.utc).isoformat()
        update_status(
            directive_id,
            "pushed",
            commit_hash=commit_hash,
            pushed_at=pushed_at,
        )

        result["success"] = True
        result["commit_hash"] = commit_hash

        _notify(
            notify_number,
            directive_id,
            success=True,
            detail=f"pushed {commit_hash[:8]} to {repo.name}/{branch}",
        )

    except subprocess.CalledProcessError as e:
        result["error"] = e.stderr or str(e)
        update_status(directive_id, "error", error=result["error"])
        _notify(notify_number, directive_id, success=False, detail=result["error"])

    return result


def push_inline_code(
    directive_id: str,
    code_blocks: dict[str, str],
    summary: str,
    *,
    base_path: Optional[Path] = None,
    repo_path: Optional[Path] = None,
    branch: str = "main",
    notify_number: Optional[str] = None,
) -> dict:
    """
    Write code_blocks to disk then push.

    Args:
        code_blocks:  {relative_path: content} mapping
        base_path:    Root inside the repo to write files (default: repo root)
    """
    repo = repo_path or repo_for_directive(directive_id)
    base = base_path or repo
    written: list[Path] = []

    for rel, content in code_blocks.items():
        dest = base / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(content, encoding="utf-8")
        written.append(dest)

    update_status(directive_id, "building", code_path=str(base))
    return push_directive_code(
        directive_id,
        written,
        summary,
        repo_path=repo,
        branch=branch,
        notify_number=notify_number,
    )


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _git(args: list[str], cwd: Path, env: dict) -> str:
    result = subprocess.run(
        ["git"] + args,
        cwd=str(cwd),
        capture_output=True,
        text=True,
        env=env,
        check=True,
    )
    return result.stdout


def _notify(number: Optional[str], directive_id: str, *, success: bool, detail: str) -> None:
    """Send a Signal confirmation back to the directive sender."""
    if not number:
        return
    try:
        from ..comms.send import send_signal
        status = "pushed" if success else "ERROR"
        msg = f"[BUNNY] directive {directive_id} — {status}: {detail}"
        send_signal(msg, recipient=number)
    except Exception:
        pass
