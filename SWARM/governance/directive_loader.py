"""
Directive Loader Module
=======================
Loads and queries the BUNNY-SWARM directive registry.

Provides programmatic access to directive metadata, status,
and compliance validation for the Calculus AI platform.
"""

import json
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


# Resolve paths relative to this module's location
_MODULE_DIR = Path(__file__).resolve().parent
_DIRECTIVES_DIR = _MODULE_DIR.parent / "directives"
_REGISTRY_PATH = _DIRECTIVES_DIR / "directive_registry.json"


@dataclass
class Directive:
    """Represents a single directive entry from the registry."""

    id: str
    name: str
    file: str
    status: str
    description: str
    planes: list[str] = field(default_factory=list)

    @property
    def file_path(self) -> Path:
        """Return the absolute path to the directive markdown file."""
        return _DIRECTIVES_DIR / self.file

    @property
    def exists(self) -> bool:
        """Check whether the directive file exists on disk."""
        return self.file_path.is_file()


@dataclass
class ComplianceResult:
    """Result of a compliance validation check."""

    passed: bool
    directive_id: str
    action: str
    reasons: list[str] = field(default_factory=list)


def _load_registry() -> dict:
    """Load the directive registry JSON from disk."""
    if not _REGISTRY_PATH.is_file():
        raise FileNotFoundError(
            f"Directive registry not found at {_REGISTRY_PATH}"
        )
    with open(_REGISTRY_PATH, "r", encoding="utf-8") as fh:
        return json.load(fh)


def _parse_directive(entry: dict) -> Directive:
    """Parse a single directive entry from the registry dict."""
    return Directive(
        id=entry["id"],
        name=entry["name"],
        file=entry["file"],
        status=entry["status"],
        description=entry["description"],
        planes=entry.get("planes", []),
    )


# ── Public API ────────────────────────────────────────────────


def list_directives() -> list[Directive]:
    """Return all directives in the registry regardless of status.

    Returns:
        A list of Directive objects.
    """
    registry = _load_registry()
    return [_parse_directive(entry) for entry in registry.get("directives", [])]


def get_directive(directive_id: str) -> Optional[Directive]:
    """Retrieve a single directive by its ID (e.g. 'BSA-01').

    Args:
        directive_id: The unique identifier of the directive.

    Returns:
        The matching Directive, or None if not found.
    """
    for directive in list_directives():
        if directive.id == directive_id:
            return directive
    return None


def get_active_directives() -> list[Directive]:
    """Return only directives whose status is 'active'.

    Returns:
        A list of active Directive objects.
    """
    return [d for d in list_directives() if d.status == "active"]


def validate_compliance(action: str, directive_id: str) -> ComplianceResult:
    """Validate whether a proposed action complies with a directive.

    This performs a structural compliance check:
    - The referenced directive must exist and be active.
    - The directive file must be present on disk.
    - The action description must not be empty.

    For deep semantic compliance (security, architecture, improvement
    governance), see ``directive_validator.py``.

    Args:
        action: A human-readable description of the proposed action.
        directive_id: The directive ID to validate against.

    Returns:
        A ComplianceResult indicating pass/fail with reasons.
    """
    reasons: list[str] = []

    if not action or not action.strip():
        reasons.append("Action description must not be empty.")

    directive = get_directive(directive_id)

    if directive is None:
        reasons.append(f"Directive '{directive_id}' not found in registry.")
        return ComplianceResult(
            passed=False,
            directive_id=directive_id,
            action=action,
            reasons=reasons,
        )

    if directive.status != "active":
        reasons.append(
            f"Directive '{directive_id}' has status '{directive.status}' "
            "(must be 'active')."
        )

    if not directive.exists:
        reasons.append(
            f"Directive file '{directive.file}' not found on disk."
        )

    passed = len(reasons) == 0
    return ComplianceResult(
        passed=passed,
        directive_id=directive_id,
        action=action,
        reasons=reasons if reasons else ["Action complies with directive."],
    )


def get_registry_version() -> str:
    """Return the version string from the directive registry.

    Returns:
        The version string (e.g. '1.0.0').
    """
    registry = _load_registry()
    return registry.get("version", "unknown")


def get_registry_updated() -> str:
    """Return the last-updated date from the directive registry.

    Returns:
        The date string (e.g. '2026-03-11').
    """
    registry = _load_registry()
    return registry.get("updated", "unknown")
