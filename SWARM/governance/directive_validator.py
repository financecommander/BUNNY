"""
Directive Validator Module
==========================
Validates proposed changes against BUNNY-SWARM directive policies.

Provides security, architecture, and improvement governance checks
for the Calculus AI platform. References BUNNY Shield for security
enforcement and BUNNY Core for architectural compliance.
"""

from dataclasses import dataclass, field
from typing import Optional


@dataclass
class ValidationResult:
    """Outcome of a directive validation check."""

    passed: bool
    check_type: str
    target: str
    reasons: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    enforced_by: str = ""


# ── Security Plane Constants ──────────────────────────────────

_PROTECTED_PLANES = frozenset({
    "security",
    "identity",
    "crypto",
    "audit",
})

_PROHIBITED_ACTIONS = frozenset({
    "disable_security",
    "bypass_authentication",
    "bypass_authorization",
    "disable_audit_logging",
    "export_private_keys",
    "modify_encryption",
    "delete_audit_logs",
    "override_shield",
    "escalate_privilege",
})

_SENSITIVE_ACTIONS = frozenset({
    "modify_agent_permissions",
    "change_routing_policy",
    "update_model_registry",
    "modify_access_controls",
    "alter_data_classification",
})

# ── Architecture Constants ────────────────────────────────────

_VALID_PLANES = frozenset({
    "security",
    "control",
    "cognition",
    "execution",
    "memory",
    "collective_intelligence",
})

_VALID_AGENT_ROLES = frozenset({
    "executive",
    "security",
    "dispatcher",
    "worker",
})

_REQUIRED_COMMUNICATION_FIELDS = frozenset({
    "source_plane",
    "target_plane",
    "source_agent",
    "message_type",
})

# ── Improvement Governance Constants ──────────────────────────

_REQUIRED_EXPERIMENT_FIELDS = frozenset({
    "hypothesis",
    "affected_modules",
    "risk_level",
    "experiment_plan",
    "expected_improvement",
})

_VALID_RISK_LEVELS = frozenset({
    "low",
    "medium",
    "high",
    "critical",
})

_REQUIRED_VALIDATION_GATES = frozenset({
    "unit_testing",
    "integration_testing",
    "security_audit",
    "performance_benchmark",
})


# ── Public API ────────────────────────────────────────────────


def validate_security_compliance(action: dict) -> ValidationResult:
    """Validate a proposed action against BSA-01/BSA-02 security policies.

    Checks enforced by BUNNY Shield:
    - Action is not in the prohibited actions list.
    - Action does not target protected security planes.
    - Sensitive actions carry required authorization metadata.

    Args:
        action: A dict describing the proposed action with keys:
            - name (str): The action identifier.
            - target_plane (str, optional): The plane being acted upon.
            - authorization (str, optional): Authorization token or reference.
            - agent_role (str, optional): Role of the requesting agent.

    Returns:
        A ValidationResult with pass/fail and detailed reasons.
    """
    reasons: list[str] = []
    warnings: list[str] = []

    action_name = action.get("name", "").lower().strip()
    target_plane = action.get("target_plane", "").lower().strip()
    authorization = action.get("authorization", "").strip()
    agent_role = action.get("agent_role", "").lower().strip()

    # Check required fields
    if not action_name:
        reasons.append("Action must include a 'name' field.")

    # Check prohibited actions
    if action_name in _PROHIBITED_ACTIONS:
        reasons.append(
            f"Action '{action_name}' is prohibited by security policy. "
            "BUNNY Shield blocks this action unconditionally."
        )

    # Check protected planes
    if target_plane in _PROTECTED_PLANES:
        reasons.append(
            f"Target plane '{target_plane}' is protected. "
            "Modifications to security, identity, crypto, and audit "
            "planes require manual authorization per BSA-03 Section 16."
        )

    # Check sensitive actions require authorization
    if action_name in _SENSITIVE_ACTIONS:
        if not authorization:
            reasons.append(
                f"Sensitive action '{action_name}' requires explicit "
                "authorization. Provide an 'authorization' reference."
            )
        else:
            warnings.append(
                f"Sensitive action '{action_name}' flagged for BUNNY Shield "
                "review. Authorization reference: {authorization}."
            )

    # Check agent role authorization
    if agent_role == "worker" and target_plane in ("security", "control"):
        reasons.append(
            f"Agent role 'worker' cannot target the '{target_plane}' plane. "
            "Workers operate exclusively within the execution plane."
        )

    passed = len(reasons) == 0
    return ValidationResult(
        passed=passed,
        check_type="security_compliance",
        target=action_name,
        reasons=reasons if reasons else ["Action passes security compliance."],
        warnings=warnings,
        enforced_by="BUNNY Shield",
    )


def validate_architecture_compliance(component: dict) -> ValidationResult:
    """Validate a proposed component against BSA-01 architecture policies.

    Checks:
    - Component is assigned to a valid architectural plane.
    - Agent role is recognized in the agent hierarchy.
    - Communication metadata includes required fields.
    - Component does not bypass the Security Plane.

    Args:
        component: A dict describing the component with keys:
            - name (str): Component name.
            - plane (str): Target architectural plane.
            - agent_role (str, optional): Role classification.
            - communication (dict, optional): Inter-plane communication
              metadata with keys: source_plane, target_plane,
              source_agent, message_type.

    Returns:
        A ValidationResult with pass/fail and detailed reasons.
    """
    reasons: list[str] = []
    warnings: list[str] = []

    component_name = component.get("name", "").strip()
    plane = component.get("plane", "").lower().strip()
    agent_role = component.get("agent_role", "").lower().strip()
    communication = component.get("communication", {})

    # Check required fields
    if not component_name:
        reasons.append("Component must include a 'name' field.")

    if not plane:
        reasons.append("Component must specify a 'plane' assignment.")
    elif plane not in _VALID_PLANES:
        reasons.append(
            f"Plane '{plane}' is not a recognized architectural plane. "
            f"Valid planes: {', '.join(sorted(_VALID_PLANES))}."
        )

    # Validate agent role
    if agent_role and agent_role not in _VALID_AGENT_ROLES:
        reasons.append(
            f"Agent role '{agent_role}' is not recognized. "
            f"Valid roles: {', '.join(sorted(_VALID_AGENT_ROLES))}."
        )

    # Validate communication metadata
    if communication:
        missing_fields = _REQUIRED_COMMUNICATION_FIELDS - set(
            communication.keys()
        )
        if missing_fields:
            reasons.append(
                "Communication metadata is missing required fields: "
                f"{', '.join(sorted(missing_fields))}."
            )

        comm_source = communication.get("source_plane", "").lower()
        comm_target = communication.get("target_plane", "").lower()

        if comm_source and comm_source not in _VALID_PLANES:
            reasons.append(
                f"Communication source_plane '{comm_source}' is not valid."
            )
        if comm_target and comm_target not in _VALID_PLANES:
            reasons.append(
                f"Communication target_plane '{comm_target}' is not valid."
            )

        # Enforce Security Plane bypass prohibition
        if (
            comm_source != "security"
            and comm_target not in ("security", "")
            and comm_source != ""
        ):
            warnings.append(
                "Communication does not originate from the security plane. "
                "Ensure the request has already passed Security Plane "
                "validation per BSA-01 Section 2."
            )

    # Check dispatcher constraints
    if agent_role == "dispatcher" and plane == "execution":
        warnings.append(
            "Dispatchers should not be deployed in the execution plane. "
            "Dispatchers classify and route; they do not execute tasks."
        )

    passed = len(reasons) == 0
    return ValidationResult(
        passed=passed,
        check_type="architecture_compliance",
        target=component_name,
        reasons=reasons if reasons else [
            "Component complies with architecture directive."
        ],
        warnings=warnings,
        enforced_by="BUNNY Core",
    )


def validate_improvement_governance(experiment: dict) -> ValidationResult:
    """Validate a proposed experiment against BSA-03 improvement governance.

    Checks:
    - Experiment includes all required metadata fields.
    - Risk level is valid.
    - Required validation gates are specified.
    - Experiment does not target protected security components.
    - BUNNY Shield approval is referenced.

    Args:
        experiment: A dict describing the experiment with keys:
            - hypothesis (str): What the experiment aims to prove.
            - affected_modules (list[str]): Modules impacted.
            - risk_level (str): low, medium, high, or critical.
            - experiment_plan (str): Description of the experiment.
            - expected_improvement (str): Quantified expected gain.
            - validation_gates (list[str]): Gates the experiment will pass.
            - shield_approval (str, optional): BUNNY Shield approval ref.

    Returns:
        A ValidationResult with pass/fail and detailed reasons.
    """
    reasons: list[str] = []
    warnings: list[str] = []

    # Check required fields
    missing_fields = _REQUIRED_EXPERIMENT_FIELDS - set(experiment.keys())
    if missing_fields:
        reasons.append(
            "Experiment is missing required fields: "
            f"{', '.join(sorted(missing_fields))}."
        )

    # Validate risk level
    risk_level = experiment.get("risk_level", "").lower().strip()
    if risk_level and risk_level not in _VALID_RISK_LEVELS:
        reasons.append(
            f"Risk level '{risk_level}' is not valid. "
            f"Valid levels: {', '.join(sorted(_VALID_RISK_LEVELS))}."
        )

    # Check validation gates
    validation_gates = {
        g.lower().strip() for g in experiment.get("validation_gates", [])
    }
    missing_gates = _REQUIRED_VALIDATION_GATES - validation_gates
    if missing_gates:
        reasons.append(
            "Experiment is missing required validation gates: "
            f"{', '.join(sorted(missing_gates))}. "
            "Per BSA-03 Section 8, all gates must be specified."
        )

    # Check affected modules for security plane components
    affected_modules = [
        m.lower().strip() for m in experiment.get("affected_modules", [])
    ]
    security_modules = [
        m for m in affected_modules
        if any(p in m for p in _PROTECTED_PLANES)
    ]
    if security_modules:
        reasons.append(
            f"Experiment targets protected modules: "
            f"{', '.join(security_modules)}. "
            "Per BSA-03 Section 16, security plane components require "
            "manual authorization and cannot be self-improved."
        )

    # Check BUNNY Shield approval for high/critical risk
    shield_approval = experiment.get("shield_approval", "").strip()
    if risk_level in ("high", "critical") and not shield_approval:
        reasons.append(
            f"Risk level '{risk_level}' requires BUNNY Shield approval. "
            "Provide a 'shield_approval' reference."
        )
    elif risk_level in ("high", "critical") and shield_approval:
        warnings.append(
            f"High-risk experiment flagged for BUNNY Shield review. "
            f"Approval reference: {shield_approval}."
        )

    # Warn on medium risk without approval
    if risk_level == "medium" and not shield_approval:
        warnings.append(
            "Medium-risk experiment submitted without BUNNY Shield approval. "
            "Approval is recommended but not required at this risk level."
        )

    passed = len(reasons) == 0
    return ValidationResult(
        passed=passed,
        check_type="improvement_governance",
        target=experiment.get("hypothesis", "unknown"),
        reasons=reasons if reasons else [
            "Experiment complies with improvement governance."
        ],
        warnings=warnings,
        enforced_by="BUNNY Shield + BUNNY Core",
    )
