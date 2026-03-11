#!/usr/bin/env python3
"""Update Bunny AI branding across swarm-mainframe codebase."""

import os
import sys

BASE = "/opt/swarm-mainframe"


def update_file(path, replacements):
    """Apply a list of (old, new) replacements to a file."""
    full = os.path.join(BASE, path)
    if not os.path.exists(full):
        print(f"  SKIP (not found): {path}")
        return False
    with open(full, "r") as f:
        content = f.read()
    original = content
    for old, new in replacements:
        if old in content:
            content = content.replace(old, new)
        else:
            print(f"  WARN: pattern not found in {path}: {old[:60]}...")
    if content != original:
        with open(full, "w") as f:
            f.write(content)
        print(f"  UPDATED: {path}")
        return True
    else:
        print(f"  NO CHANGES: {path}")
        return False


# ============================================================
# 1. slack_commands.py — Major system prompt rewrite
# ============================================================
print("\n=== Updating slack_commands.py ===")

slack_replacements = [
    # Identity line
    (
        'You are BUNNY \u2014 Secure AI Operations Assistant '
        'embedded in the Calculus Holdings swarm system.',
        'You are Bunny Alpha \u2014 OpenClaw Infrastructure Assistant '
        'within the Bunny AI system (Calculus Holdings).'
    ),
    # Personality
    (
        'Identity: precise, calm, technically fluent, security-aware, '
        'execution-focused, non-dramatic, highly structured. '
        'You are a trusted operator node within the swarm, not a chatbot.',
        'Identity: technical, decisive, systems-focused. '
        'You have full access to system infrastructure, VM orchestration, '
        'swarm coordination, environment configuration, and execution oversight. '
        'You are an infrastructure authority, not a chatbot.'
    ),
    # Add assistant architecture before communication style
    (
        '"Communication style:\\n"',
        '"BUNNY AI ASSISTANT ARCHITECTURE:\\n"\n'
        '        "Three OpenClaw assistants serve the Calculus team:\\n"\n'
        '        "1. Jack - Primary assistant (helpful, friendly, collaborative)\\n"\n'
        '        "2. Joyceann - Operations assistant (concise, pragmatic, efficient)\\n"\n'
        '        "3. Bunny Alpha (YOU) - Infrastructure assistant (technical, decisive)\\n\\n"\n'
        '        "Authority: Jack/Joyceann interpret and dispatch. Bunny Alpha has full infrastructure authority.\\n"\n'
        '        "Execution: User -> AI Portal/Slack -> OpenClaw Assistant -> SWARM -> Provider -> Results\\n\\n"\n'
        '        "Communication style:\\n"'
    ),
    # Operational modes
    (
        '"Operational modes:\\n"',
        '"Operational modes (Bunny Alpha):\\n"'
    ),
    # Domains
    (
        'Domains: financial analysis, real estate leads, business operations, ',
        'Domains: system infrastructure, swarm orchestration, VM management, '
    ),
    (
        'research, automation, data pipelines, system architecture.',
        'deployment, monitoring, data pipelines, system architecture, business operations.'
    ),
    # Help text
    (
        'Ask Bunny anything',
        'Ask Bunny Alpha anything'
    ),
    (
        ':rabbit2: *BUNNY \u2014 Secure AI Operations Assistant*',
        ':rabbit2: *Bunny Alpha \u2014 Bunny AI Infrastructure Assistant*'
    ),
    (
        'also route to BUNNY.',
        'also route to Bunny Alpha.'
    ),
    # Status label
    (
        'SWARM STATUS:',
        'BUNNY AI STATUS:'
    ),
    # Metadata key
    (
        '"bunny_mode"',
        '"bunny_alpha_mode"'
    ),
    # Closing line
    (
        'Ready for the next task.',
        'Bunny Alpha ready.'
    ),
    # Comment headers
    (
        '# BUNNY \u2014 Interactive AI Agent',
        '# BUNNY ALPHA \u2014 Bunny AI Infrastructure Agent'
    ),
    (
        '# SWARM CHAT FALLBACK (routes to Bunny)',
        '# SWARM CHAT FALLBACK (routes to Bunny Alpha)'
    ),
    # Docstrings
    (
        'Route free-text through Bunny with',
        'Route free-text through Bunny Alpha with'
    ),
    (
        'Run a Bunny query with',
        'Run a Bunny Alpha query with'
    ),
    (
        'Build context block for Bunny',
        'Build context block for Bunny Alpha'
    ),
    (
        'Bunny gets live swarm stats',
        'Bunny Alpha gets live swarm stats'
    ),
    # Error messages
    (
        "Bunny couldn't generate",
        "Bunny Alpha couldn't generate"
    ),
    (
        ':x: Bunny error:',
        ':x: Bunny Alpha error:'
    ),
    # Agent loop help
    (
        'Bunny will iteratively call tools',
        'Bunny Alpha will iteratively call tools'
    ),
]

update_file("openclaw/slack_commands.py", slack_replacements)

# ============================================================
# 2. messaging_bridge.py — Remove codespace reference
# ============================================================
print("\n=== Updating messaging_bridge.py ===")
update_file("openclaw/messaging_bridge.py", [
    (
        'Codespace: vigilant-engine-x564p6x4vqgqc64jj',
        'Bunny AI Infrastructure'
    ),
])

# ============================================================
# 3. slack_connector.py — Module docstring
# ============================================================
print("\n=== Updating slack_connector.py ===")
update_file("openclaw/slack_connector.py", [
    (
        'OpenClaw Slack Connector',
        'OpenClaw Slack Connector (Bunny AI)'
    ),
    (
        'Bidirectional Slack integration for the swarm orchestrator and OpenClaw tools:',
        'Bidirectional Slack integration for Bunny AI (swarm orchestrator + OpenClaw):'
    ),
])

# ============================================================
# 4. main.py — Startup log branding
# ============================================================
print("\n=== Updating main.py ===")
update_file("main.py", [
    (
        '"swarm_ready"',
        '"bunny_ai_ready"'
    ),
    (
        '"swarm_shutting_down"',
        '"bunny_ai_shutting_down"'
    ),
])

print("\n=== All branding updates complete! ===")
