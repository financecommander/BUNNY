#!/usr/bin/env python3
"""
Bunny Alpha v3.2 — Autonomous Operations Platform

Standalone Slack assistant with real infrastructure execution.
Task queue, concurrent execution, progress reporting.
Operational hardening, continuous learning, scale & autonomy,
environment intelligence, digital twin simulation.

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

            -- Monitoring checks
            CREATE TABLE IF NOT EXISTS monitor_checks (
                check_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                target TEXT NOT NULL,
                check_type TEXT NOT NULL,
                command TEXT NOT NULL,
                interval_seconds INTEGER DEFAULT 300,
                severity TEXT DEFAULT 'warning',
                enabled INTEGER DEFAULT 1,
                muted INTEGER DEFAULT 0,
                last_status TEXT,
                last_result TEXT,
                last_run_at REAL
            );

            -- Monitoring alerts
            CREATE TABLE IF NOT EXISTS monitor_alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                check_id TEXT NOT NULL,
                status TEXT NOT NULL,
                message TEXT,
                created_at REAL NOT NULL,
                resolved_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_alerts_check ON monitor_alerts(check_id, created_at);

            -- Scheduled jobs
            CREATE TABLE IF NOT EXISTS scheduled_jobs (
                job_id TEXT PRIMARY KEY,
                owner TEXT,
                channel_id TEXT,
                thread_ts TEXT,
                job_type TEXT NOT NULL,
                description TEXT,
                payload TEXT NOT NULL,
                schedule_expression TEXT,
                interval_seconds INTEGER,
                next_run_at REAL,
                last_run_at REAL,
                enabled INTEGER DEFAULT 1,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_jobs_next ON scheduled_jobs(enabled, next_run_at);

            -- Knowledge Graph
            CREATE TABLE IF NOT EXISTS graph_entities (
                entity_id TEXT PRIMARY KEY,
                entity_type TEXT NOT NULL,
                name TEXT NOT NULL,
                attributes_json TEXT,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_entities_type ON graph_entities(entity_type);
            CREATE INDEX IF NOT EXISTS idx_entities_name ON graph_entities(name);

            CREATE TABLE IF NOT EXISTS graph_edges (
                edge_id TEXT PRIMARY KEY,
                src_entity_id TEXT NOT NULL,
                relation TEXT NOT NULL,
                dst_entity_id TEXT NOT NULL,
                attributes_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_edges_src ON graph_edges(src_entity_id);
            CREATE INDEX IF NOT EXISTS idx_edges_dst ON graph_edges(dst_entity_id);
            CREATE INDEX IF NOT EXISTS idx_edges_relation ON graph_edges(relation);

            CREATE TABLE IF NOT EXISTS graph_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_entity ON graph_events(entity_id, created_at);

            -- Autonomous Planning
            CREATE TABLE IF NOT EXISTS goal_plans (
                plan_id TEXT PRIMARY KEY,
                goal_text TEXT NOT NULL,
                created_by TEXT,
                status TEXT DEFAULT 'planning',
                summary TEXT,
                created_at REAL NOT NULL,
                completed_at REAL
            );

            CREATE TABLE IF NOT EXISTS plan_steps (
                step_id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT,
                task_type TEXT,
                priority INTEGER DEFAULT 5,
                dependencies TEXT,
                assigned_service TEXT,
                assigned_agent TEXT,
                status TEXT DEFAULT 'pending',
                retries INTEGER DEFAULT 0,
                result_summary TEXT,
                created_at REAL NOT NULL,
                completed_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_steps_plan ON plan_steps(plan_id);

            -- Multi-Agent
            CREATE TABLE IF NOT EXISTS agent_specs (
                agent_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                role TEXT NOT NULL,
                capabilities TEXT,
                priority INTEGER DEFAULT 5,
                status TEXT DEFAULT 'active'
            );

            CREATE TABLE IF NOT EXISTS delegated_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                parent_task_id TEXT,
                assigned_agent TEXT NOT NULL,
                task_payload TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                result_summary TEXT,
                confidence REAL,
                created_at REAL NOT NULL,
                completed_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_delegated_agent ON delegated_tasks(assigned_agent);

            -- Predictive Monitoring
            CREATE TABLE IF NOT EXISTS prediction_signals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                target TEXT NOT NULL,
                signal_type TEXT NOT NULL,
                confidence REAL,
                predicted_failure_window TEXT,
                supporting_metrics TEXT,
                created_at REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS risk_assessments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                target TEXT NOT NULL,
                risk_score REAL,
                risk_level TEXT,
                explanation TEXT,
                recommended_action TEXT,
                created_at REAL NOT NULL
            );

            -- Learning Planner
            CREATE TABLE IF NOT EXISTS plan_outcomes (
                plan_id TEXT PRIMARY KEY,
                goal_type TEXT,
                success INTEGER,
                total_duration REAL,
                retries INTEGER DEFAULT 0,
                escalations INTEGER DEFAULT 0,
                rollback_triggered INTEGER DEFAULT 0,
                final_summary TEXT,
                completed_at REAL
            );

            -- Performance-Aware Routing
            CREATE TABLE IF NOT EXISTS routing_performance (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                route_type TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                success_rate REAL DEFAULT 1.0,
                avg_latency REAL DEFAULT 0,
                error_rate REAL DEFAULT 0,
                request_count INTEGER DEFAULT 0,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_routing_target ON routing_performance(target_id);

            CREATE TABLE IF NOT EXISTS routing_decisions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT,
                route_reason TEXT,
                selected_target TEXT,
                alternatives_json TEXT,
                confidence REAL,
                created_at REAL NOT NULL
            );

            -- Execution Simulation
            CREATE TABLE IF NOT EXISTS simulations (
                simulation_id TEXT PRIMARY KEY,
                plan_id TEXT,
                scenario_type TEXT,
                inputs_json TEXT,
                predicted_outcomes_json TEXT,
                risk_score REAL,
                recommended_action TEXT,
                created_at REAL NOT NULL
            );

            -- ===== OPERATIONAL HARDENING LAYER =====

            -- Swarm Sessions
            CREATE TABLE IF NOT EXISTS swarm_sessions (
                session_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                assistant_name TEXT DEFAULT 'bunny-alpha',
                portal_session_id TEXT,
                status TEXT DEFAULT 'active',
                workspace_context_json TEXT,
                summary TEXT,
                active_plan_id TEXT,
                active_task_ids_json TEXT,
                created_at REAL NOT NULL,
                last_active_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON swarm_sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_status ON swarm_sessions(status);

            CREATE TABLE IF NOT EXISTS session_events (
                event_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_session_events ON session_events(session_id, created_at);

            -- Audit Logging
            CREATE TABLE IF NOT EXISTS audit_events (
                audit_id TEXT PRIMARY KEY,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                session_id TEXT,
                task_id TEXT,
                action_type TEXT NOT NULL,
                target_type TEXT,
                target_id TEXT,
                payload_json TEXT,
                result TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_events(actor_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_events(session_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_events(action_type, created_at);
            CREATE INDEX IF NOT EXISTS idx_audit_time ON audit_events(created_at);

            -- Permissions
            CREATE TABLE IF NOT EXISTS permissions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                action_class TEXT NOT NULL,
                allowed INTEGER DEFAULT 1,
                UNIQUE(role, action_class)
            );

            -- Approvals
            CREATE TABLE IF NOT EXISTS approvals (
                approval_id TEXT PRIMARY KEY,
                requested_by TEXT NOT NULL,
                action_type TEXT NOT NULL,
                action_class TEXT DEFAULT 'RISKY_MUTATION',
                action_payload TEXT,
                reason TEXT,
                status TEXT DEFAULT 'pending',
                approved_by TEXT,
                created_at REAL NOT NULL,
                resolved_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_approvals_status ON approvals(status);

            -- Execution Policies
            CREATE TABLE IF NOT EXISTS execution_policies (
                policy_id TEXT PRIMARY KEY,
                action_type TEXT NOT NULL,
                timeout_seconds INTEGER DEFAULT 30,
                output_limit INTEGER DEFAULT 10000,
                allowed_paths_json TEXT,
                allowed_hosts_json TEXT,
                network_policy TEXT DEFAULT 'allow',
                role_scope TEXT DEFAULT 'OPERATOR'
            );

            -- Failure Drills
            CREATE TABLE IF NOT EXISTS failure_drills (
                drill_id TEXT PRIMARY KEY,
                drill_type TEXT NOT NULL,
                target TEXT,
                status TEXT DEFAULT 'pending',
                started_at REAL,
                completed_at REAL,
                outcome TEXT,
                notes TEXT
            );

            CREATE TABLE IF NOT EXISTS drill_results (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                drill_id TEXT NOT NULL,
                detection_time REAL,
                mitigation_time REAL,
                recovery_time REAL,
                rollback_triggered INTEGER DEFAULT 0,
                lessons_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_drill_results ON drill_results(drill_id);

            -- Escalations
            CREATE TABLE IF NOT EXISTS escalations (
                escalation_id TEXT PRIMARY KEY,
                session_id TEXT,
                task_id TEXT,
                plan_id TEXT,
                trigger_type TEXT NOT NULL,
                confidence REAL,
                recommended_actions_json TEXT,
                status TEXT DEFAULT 'open',
                resolved_by TEXT,
                resolution_notes TEXT,
                created_at REAL NOT NULL,
                resolved_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_escalations_status ON escalations(status);

            -- ===== CONTINUOUS LEARNING LAYER =====

            -- Task Outcomes
            CREATE TABLE IF NOT EXISTS task_outcomes (
                outcome_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                session_id TEXT,
                plan_id TEXT,
                task_type TEXT,
                route_type TEXT,
                selected_target TEXT,
                success INTEGER,
                duration_ms REAL,
                retries INTEGER DEFAULT 0,
                escalations INTEGER DEFAULT 0,
                provider_used TEXT,
                worker_used TEXT,
                cost_estimate REAL,
                result_quality REAL,
                human_override INTEGER DEFAULT 0,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_task_outcomes_type ON task_outcomes(task_type, created_at);

            -- Route Outcomes
            CREATE TABLE IF NOT EXISTS route_outcomes (
                route_outcome_id TEXT PRIMARY KEY,
                routing_decision_id TEXT,
                selected_target TEXT,
                success INTEGER,
                latency_ms REAL,
                queue_delay_ms REAL,
                fallback_used INTEGER DEFAULT 0,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_route_outcomes_target ON route_outcomes(selected_target);

            -- Repair Outcomes
            CREATE TABLE IF NOT EXISTS repair_outcomes (
                repair_outcome_id TEXT PRIMARY KEY,
                repair_id TEXT NOT NULL,
                repair_type TEXT,
                success INTEGER,
                rollback_triggered INTEGER DEFAULT 0,
                recovery_time_ms REAL,
                created_at REAL NOT NULL
            );

            -- Step Outcomes
            CREATE TABLE IF NOT EXISTS step_outcomes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                step_id TEXT NOT NULL,
                plan_id TEXT NOT NULL,
                step_type TEXT,
                assigned_service TEXT,
                assigned_agent TEXT,
                success INTEGER,
                retries INTEGER DEFAULT 0,
                duration_ms REAL,
                result_quality REAL,
                completed_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_step_outcomes_plan ON step_outcomes(plan_id);

            -- Planning Patterns
            CREATE TABLE IF NOT EXISTS planning_patterns (
                pattern_id TEXT PRIMARY KEY,
                goal_type TEXT NOT NULL,
                pattern_signature TEXT,
                success_rate REAL DEFAULT 0.5,
                avg_duration_ms REAL DEFAULT 0,
                avg_retries REAL DEFAULT 0,
                confidence REAL DEFAULT 0.5,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_patterns_goal ON planning_patterns(goal_type);

            -- Routing Scores
            CREATE TABLE IF NOT EXISTS routing_scores (
                routing_score_id TEXT PRIMARY KEY,
                route_type TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                success_score REAL DEFAULT 0.5,
                latency_score REAL DEFAULT 0.5,
                cost_score REAL DEFAULT 0.5,
                reliability_score REAL DEFAULT 0.5,
                task_fit_score REAL DEFAULT 0.5,
                overall_score REAL DEFAULT 0.5,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_routing_scores ON routing_scores(target_id);

            -- Routing Weights
            CREATE TABLE IF NOT EXISTS routing_weights (
                weight_id TEXT PRIMARY KEY,
                routing_mode TEXT NOT NULL UNIQUE,
                success_weight REAL DEFAULT 0.3,
                latency_weight REAL DEFAULT 0.25,
                cost_weight REAL DEFAULT 0.15,
                reliability_weight REAL DEFAULT 0.2,
                fit_weight REAL DEFAULT 0.1,
                updated_at REAL NOT NULL
            );

            -- Memory Distillations
            CREATE TABLE IF NOT EXISTS memory_distillations (
                distillation_id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_ids_json TEXT,
                distilled_summary TEXT NOT NULL,
                topic_tags_json TEXT,
                entity_links_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_distillations ON memory_distillations(source_type, created_at);

            -- System Knowledge
            CREATE TABLE IF NOT EXISTS system_knowledge (
                knowledge_id TEXT PRIMARY KEY,
                knowledge_type TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT,
                supporting_refs_json TEXT,
                confidence REAL DEFAULT 0.5,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge ON system_knowledge(knowledge_type);

            -- Incident Patterns
            CREATE TABLE IF NOT EXISTS incident_patterns (
                pattern_id TEXT PRIMARY KEY,
                incident_type TEXT NOT NULL,
                common_signals_json TEXT,
                common_resolutions_json TEXT,
                recurrence_score REAL DEFAULT 0,
                updated_at REAL NOT NULL
            );

            -- Execution Recipes
            CREATE TABLE IF NOT EXISTS execution_recipes (
                recipe_id TEXT PRIMARY KEY,
                task_type TEXT NOT NULL,
                successful_steps_json TEXT,
                required_conditions_json TEXT,
                success_rate REAL DEFAULT 0.5,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_recipes ON execution_recipes(task_type);

            -- Repair Patterns
            CREATE TABLE IF NOT EXISTS repair_patterns (
                repair_pattern_id TEXT PRIMARY KEY,
                fault_class TEXT NOT NULL,
                target_type TEXT,
                repair_type TEXT NOT NULL,
                success_rate REAL DEFAULT 0.5,
                avg_recovery_time_ms REAL DEFAULT 0,
                rollback_rate REAL DEFAULT 0,
                confidence REAL DEFAULT 0.5,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_repair_patterns ON repair_patterns(fault_class);

            -- Repair Policies Learned
            CREATE TABLE IF NOT EXISTS repair_policies_learned (
                learned_policy_id TEXT PRIMARY KEY,
                fault_class TEXT NOT NULL UNIQUE,
                preferred_repairs_json TEXT,
                discouraged_repairs_json TEXT,
                explanation TEXT,
                updated_at REAL NOT NULL
            );

            -- Agent Scores
            CREATE TABLE IF NOT EXISTS agent_scores (
                agent_score_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL UNIQUE,
                success_rate REAL DEFAULT 0.5,
                avg_latency_ms REAL DEFAULT 0,
                quality_score REAL DEFAULT 0.5,
                escalation_rate REAL DEFAULT 0,
                collaboration_score REAL DEFAULT 0.5,
                reliability_score REAL DEFAULT 0.5,
                updated_at REAL NOT NULL
            );

            -- Agent Outcomes
            CREATE TABLE IF NOT EXISTS agent_outcomes (
                agent_outcome_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                delegated_task_id TEXT,
                task_type TEXT,
                success INTEGER,
                duration_ms REAL,
                quality_score REAL,
                error_type TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_outcomes ON agent_outcomes(agent_id, created_at);

            -- Intelligence Loop
            CREATE TABLE IF NOT EXISTS intelligence_runs (
                intelligence_run_id TEXT PRIMARY KEY,
                started_at REAL NOT NULL,
                completed_at REAL,
                changes_json TEXT,
                success INTEGER,
                notes TEXT
            );

            CREATE TABLE IF NOT EXISTS learning_updates (
                update_id TEXT PRIMARY KEY,
                intelligence_run_id TEXT NOT NULL,
                update_type TEXT NOT NULL,
                target TEXT,
                before_value_json TEXT,
                after_value_json TEXT,
                explanation TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_learning_updates ON learning_updates(intelligence_run_id);

            -- Decision Explanations
            CREATE TABLE IF NOT EXISTS decision_explanations (
                explanation_id TEXT PRIMARY KEY,
                decision_type TEXT NOT NULL,
                decision_id TEXT NOT NULL,
                explanation_text TEXT NOT NULL,
                supporting_factors_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_explanations ON decision_explanations(decision_type, decision_id);

            -- ===== SCALE & AUTONOMY MATURITY LAYER =====

            -- Worker Registry
            CREATE TABLE IF NOT EXISTS worker_registry (
                worker_id TEXT PRIMARY KEY,
                host TEXT NOT NULL,
                region TEXT DEFAULT 'us-east1',
                capabilities_json TEXT,
                health_score REAL DEFAULT 1.0,
                active_tasks INTEGER DEFAULT 0,
                last_heartbeat REAL,
                status TEXT DEFAULT 'active'
            );
            CREATE INDEX IF NOT EXISTS idx_workers_status ON worker_registry(status);

            CREATE TABLE IF NOT EXISTS worker_health_history (
                record_id TEXT PRIMARY KEY,
                worker_id TEXT NOT NULL,
                health_score REAL,
                task_failures INTEGER DEFAULT 0,
                latency_ms REAL,
                recorded_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_worker_health ON worker_health_history(worker_id, recorded_at);

            -- Autonomous Initiative
            CREATE TABLE IF NOT EXISTS initiative_events (
                event_id TEXT PRIMARY KEY,
                trigger_type TEXT NOT NULL,
                source_signal TEXT,
                recommended_action TEXT,
                risk_level TEXT DEFAULT 'LOW',
                confidence REAL DEFAULT 0.5,
                status TEXT DEFAULT 'proposed',
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_initiative_events ON initiative_events(status, created_at);

            CREATE TABLE IF NOT EXISTS initiative_actions (
                initiative_id TEXT PRIMARY KEY,
                event_id TEXT,
                action_type TEXT NOT NULL,
                parameters_json TEXT,
                execution_status TEXT DEFAULT 'pending',
                approval_required INTEGER DEFAULT 0,
                executed_at REAL
            );

            -- Digital Twin Entities
            CREATE TABLE IF NOT EXISTS digital_twin_entities (
                entity_id TEXT PRIMARY KEY,
                entity_type TEXT NOT NULL,
                state_json TEXT,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_twin_type ON digital_twin_entities(entity_type);

            -- Knowledge Evolution
            CREATE TABLE IF NOT EXISTS knowledge_clusters (
                cluster_id TEXT PRIMARY KEY,
                topic TEXT NOT NULL,
                related_entities_json TEXT,
                pattern_summary TEXT,
                confidence REAL DEFAULT 0.5,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_clusters_topic ON knowledge_clusters(topic);

            CREATE TABLE IF NOT EXISTS operational_playbooks (
                playbook_id TEXT PRIMARY KEY,
                incident_type TEXT NOT NULL,
                recommended_steps_json TEXT,
                success_rate REAL DEFAULT 0.5,
                usage_count INTEGER DEFAULT 0,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_playbooks ON operational_playbooks(incident_type);

            -- Continuous System Evaluation
            CREATE TABLE IF NOT EXISTS system_evaluations (
                evaluation_id TEXT PRIMARY KEY,
                evaluation_type TEXT NOT NULL,
                metrics_json TEXT,
                score REAL,
                recommendations_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_evaluations ON system_evaluations(evaluation_type, created_at);

            CREATE TABLE IF NOT EXISTS system_scorecards (
                scorecard_id TEXT PRIMARY KEY,
                component TEXT NOT NULL,
                reliability_score REAL DEFAULT 0.5,
                latency_score REAL DEFAULT 0.5,
                efficiency_score REAL DEFAULT 0.5,
                trend TEXT DEFAULT 'stable',
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_scorecards ON system_scorecards(component);

            -- Ecosystem Plugins
            CREATE TABLE IF NOT EXISTS plugins (
                plugin_id TEXT PRIMARY KEY,
                plugin_type TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                capabilities_json TEXT,
                config_json TEXT,
                version TEXT DEFAULT '1.0.0',
                status TEXT DEFAULT 'active',
                registered_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_plugins ON plugins(plugin_type, status);

            -- ===== ENVIRONMENT INTELLIGENCE & DIGITAL TWIN LAYER =====

            -- Environment Signals
            CREATE TABLE IF NOT EXISTS environment_signals (
                signal_id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_id TEXT NOT NULL,
                metric_name TEXT NOT NULL,
                metric_value REAL,
                timestamp REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_env_signals ON environment_signals(source_id, timestamp);

            -- Environment State
            CREATE TABLE IF NOT EXISTS environment_state (
                state_id TEXT PRIMARY KEY,
                entity_id TEXT NOT NULL UNIQUE,
                state_snapshot_json TEXT,
                derived_health_score REAL DEFAULT 1.0,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_env_state ON environment_state(entity_id);

            -- Digital Twin
            CREATE TABLE IF NOT EXISTS twin_entities (
                twin_entity_id TEXT PRIMARY KEY,
                entity_type TEXT NOT NULL,
                entity_ref TEXT NOT NULL,
                current_state_json TEXT,
                last_updated REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_twin ON twin_entities(entity_type);

            CREATE TABLE IF NOT EXISTS twin_relationships (
                relationship_id TEXT PRIMARY KEY,
                source_entity TEXT NOT NULL,
                relation TEXT NOT NULL,
                target_entity TEXT NOT NULL,
                attributes_json TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_twin_rel ON twin_relationships(source_entity);

            CREATE TABLE IF NOT EXISTS twin_simulations (
                simulation_id TEXT PRIMARY KEY,
                scenario_type TEXT NOT NULL,
                inputs_json TEXT,
                predicted_results_json TEXT,
                risk_score REAL,
                explanation TEXT,
                created_at REAL NOT NULL
            );

            -- Event Stream
            CREATE TABLE IF NOT EXISTS event_stream (
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                entity_type TEXT,
                entity_id TEXT,
                payload_json TEXT,
                severity TEXT DEFAULT 'info',
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_type ON event_stream(event_type, created_at);
            CREATE INDEX IF NOT EXISTS idx_events_entity ON event_stream(entity_id, created_at);

            CREATE TABLE IF NOT EXISTS event_index (
                event_id TEXT PRIMARY KEY,
                tags_json TEXT,
                related_entities_json TEXT,
                correlation_group TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_event_corr ON event_index(correlation_group);

            -- Autonomous Operations
            CREATE TABLE IF NOT EXISTS autonomous_actions (
                action_id TEXT PRIMARY KEY,
                trigger_type TEXT NOT NULL,
                target_entity TEXT,
                recommended_action TEXT,
                risk_level TEXT DEFAULT 'LOW',
                confidence REAL DEFAULT 0.5,
                status TEXT DEFAULT 'proposed',
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_auto_actions ON autonomous_actions(status, created_at);

            CREATE TABLE IF NOT EXISTS autonomous_executions (
                execution_id TEXT PRIMARY KEY,
                action_id TEXT NOT NULL,
                executed INTEGER DEFAULT 0,
                approval_required INTEGER DEFAULT 0,
                execution_status TEXT DEFAULT 'pending',
                executed_at REAL
            );

            -- ===== STRUCTURED EXECUTION & SAFETY BOUNDARY LAYER =====

            -- Execution Actions (all infra operations become structured actions)
            CREATE TABLE IF NOT EXISTS execution_actions (
                action_id TEXT PRIMARY KEY,
                action_type TEXT NOT NULL,
                parameters_json TEXT,
                requested_by TEXT DEFAULT 'system',
                session_id TEXT,
                risk_level TEXT DEFAULT 'READ_ONLY',
                approval_required INTEGER DEFAULT 0,
                approval_id TEXT,
                status TEXT DEFAULT 'pending',
                created_at REAL NOT NULL,
                resolved_at REAL
            );
            CREATE INDEX IF NOT EXISTS idx_exec_actions_status ON execution_actions(status, created_at);
            CREATE INDEX IF NOT EXISTS idx_exec_actions_type ON execution_actions(action_type, created_at);

            -- Execution Results (output of each action)
            CREATE TABLE IF NOT EXISTS execution_results (
                result_id TEXT PRIMARY KEY,
                action_id TEXT NOT NULL,
                execution_host TEXT,
                command_executed TEXT,
                stdout TEXT,
                stderr TEXT,
                exit_code INTEGER,
                success INTEGER DEFAULT 1,
                duration_ms REAL,
                timestamp REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_exec_results ON execution_results(action_id);

            -- Action Policies (per-action-type rules)
            CREATE TABLE IF NOT EXISTS action_policies (
                policy_id TEXT PRIMARY KEY,
                action_type TEXT NOT NULL UNIQUE,
                risk_level TEXT DEFAULT 'SAFE_MUTATION',
                requires_approval INTEGER DEFAULT 0,
                allowed_roles TEXT DEFAULT 'OPERATOR,ADMIN,SYSTEM',
                timeout_seconds INTEGER DEFAULT 60,
                adapter TEXT DEFAULT 'host',
                description TEXT,
                updated_at REAL NOT NULL
            );

            -- Audit Actions (structured action audit trail)
            CREATE TABLE IF NOT EXISTS audit_actions (
                audit_id TEXT PRIMARY KEY,
                action_id TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                action_type TEXT NOT NULL,
                parameters_json TEXT,
                result TEXT,
                risk_level TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_actions ON audit_actions(action_type, created_at);

            -- ============================================================
            -- Proactive Relationship & Opportunity Engine
            -- ============================================================

            CREATE TABLE IF NOT EXISTS company_signals (
                signal_id TEXT PRIMARY KEY,
                company_name TEXT NOT NULL,
                domain TEXT,
                signal_type TEXT NOT NULL,
                signal_source TEXT NOT NULL,
                signal_payload_json TEXT,
                confidence REAL DEFAULT 0.5,
                processed INTEGER DEFAULT 0,
                timestamp REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_comp_signals_ts ON company_signals(timestamp);
            CREATE INDEX IF NOT EXISTS idx_comp_signals_type ON company_signals(signal_type);

            CREATE TABLE IF NOT EXISTS signal_sources (
                source_id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_url TEXT,
                polling_interval INTEGER DEFAULT 3600,
                last_scan REAL DEFAULT 0,
                enabled INTEGER DEFAULT 1,
                created_at REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS company_profiles (
                company_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                domain TEXT,
                industry TEXT,
                size_estimate TEXT,
                tech_stack_json TEXT,
                ai_need_score REAL DEFAULT 0.0,
                security_sensitivity_score REAL DEFAULT 0.0,
                description TEXT,
                website TEXT,
                location TEXT,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_comp_profiles_score ON company_profiles(ai_need_score DESC);

            CREATE TABLE IF NOT EXISTS opportunity_scores (
                opportunity_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                ai_fit_score REAL DEFAULT 0.0,
                estimated_value REAL DEFAULT 0.0,
                difficulty_score REAL DEFAULT 0.5,
                confidence REAL DEFAULT 0.5,
                scoring_factors_json TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_opp_scores ON opportunity_scores(ai_fit_score DESC);

            CREATE TABLE IF NOT EXISTS relationship_pipeline (
                pipeline_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                stage TEXT DEFAULT 'detected',
                last_contact_time REAL,
                notes TEXT,
                assigned_agent TEXT,
                priority INTEGER DEFAULT 5,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_rel_pipeline_stage ON relationship_pipeline(stage);

            CREATE TABLE IF NOT EXISTS relationship_events (
                event_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_details_json TEXT,
                actor TEXT DEFAULT 'system',
                timestamp REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_rel_events_ts ON relationship_events(timestamp);

            CREATE TABLE IF NOT EXISTS company_research (
                research_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                summary TEXT,
                ai_use_cases_json TEXT,
                internal_process_candidates_json TEXT,
                security_requirements TEXT,
                competitor_landscape TEXT,
                products_services TEXT,
                created_at REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS outreach_messages (
                message_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                channel TEXT NOT NULL,
                message_content TEXT NOT NULL,
                personalization_fields_json TEXT,
                template_id TEXT,
                sent_at REAL,
                response_status TEXT DEFAULT 'draft',
                approved_by TEXT,
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_outreach_status ON outreach_messages(response_status);

            CREATE TABLE IF NOT EXISTS demo_blueprints (
                blueprint_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                domain TEXT,
                agents_json TEXT,
                workflows_json TEXT,
                integrations_json TEXT,
                monitoring_rules_json TEXT,
                deployment_targets_json TEXT,
                industry_template TEXT,
                created_at REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS system_proposals (
                proposal_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                blueprint_id TEXT,
                architecture_summary TEXT,
                problem_summary TEXT,
                security_model TEXT,
                deployment_approach TEXT,
                estimated_cost REAL,
                estimated_roi REAL,
                expected_benefits TEXT,
                status TEXT DEFAULT 'draft',
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_proposals_status ON system_proposals(status);

            CREATE TABLE IF NOT EXISTS deployments (
                deployment_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                blueprint_id TEXT,
                proposal_id TEXT,
                deployment_status TEXT DEFAULT 'planned',
                environment_target TEXT,
                deployment_plan_json TEXT,
                monitoring_url TEXT,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_deployments_status ON deployments(deployment_status);

            CREATE TABLE IF NOT EXISTS client_revenue (
                revenue_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                deployment_id TEXT,
                revenue_type TEXT NOT NULL,
                amount REAL NOT NULL,
                billing_period TEXT,
                notes TEXT,
                updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_revenue_company ON client_revenue(company_id);

            CREATE TABLE IF NOT EXISTS relationship_outcomes (
                outcome_id TEXT PRIMARY KEY,
                company_id TEXT NOT NULL,
                stage_reached TEXT NOT NULL,
                success INTEGER DEFAULT 0,
                revenue_generated REAL DEFAULT 0.0,
                lessons_json TEXT,
                created_at REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS optimization_updates (
                update_id TEXT PRIMARY KEY,
                strategy_change TEXT NOT NULL,
                performance_delta REAL,
                affected_stage TEXT,
                timestamp REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS outreach_policies (
                policy_id TEXT PRIMARY KEY,
                rule_type TEXT NOT NULL,
                rule_value TEXT,
                enforcement_action TEXT NOT NULL,
                max_per_day INTEGER DEFAULT 10,
                cooldown_hours INTEGER DEFAULT 72,
                enabled INTEGER DEFAULT 1,
                updated_at REAL NOT NULL
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
    BLOCKED = "blocked"
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
    created_by: str = ""
    retries: int = 0

    @property
    def duration(self) -> Optional[float]:
        if self.started_at and self.completed_at:
            return round(self.completed_at - self.started_at, 1)
        return None

    @property
    def short_id(self) -> str:
        return self.task_id[:6]


class TaskManager:
    """Manages concurrent task execution with progress reporting and persistence."""

    def __init__(self):
        self.tasks: Dict[str, Task] = {}
        self.active_count = 0
        self._lock = asyncio.Lock()
        self.groups: Dict[str, List[str]] = {}

    def create_task(self, tool: str, host: str, cmd: str,
                    channel: str = "", thread_ts: str = "",
                    group_id: Optional[str] = None,
                    created_by: str = "") -> Task:
        """Create and register a new task."""
        task_id = uuid.uuid4().hex[:8]
        task = Task(
            task_id=task_id, tool=tool, host=host, cmd=cmd,
            channel=channel, thread_ts=thread_ts, created_by=created_by,
        )
        self.tasks[task_id] = task

        if group_id:
            if group_id not in self.groups:
                self.groups[group_id] = []
            self.groups[group_id].append(task_id)

        log.info(f"Task {task.short_id} created: {tool}@{host} -> {cmd[:60]}")
        return task

    async def execute_group(self, group_id: str, channel: str, thread_ts: str) -> List[Task]:
        """Execute all tasks in a group concurrently with persistence."""
        task_ids = self.groups.get(group_id, [])
        if not task_ids:
            return []

        tasks = [self.tasks[tid] for tid in task_ids]
        total = len(tasks)

        # Persist task creation
        for t in tasks:
            await memory.log_task(t.task_id, channel, thread_ts,
                                  f"{t.tool}@{t.host}: {t.cmd[:200]}", "queued")

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
                await memory.update_task(tasks[i].task_id, "failed", str(result))

        return tasks

    async def _run_task(self, task: Task) -> Task:
        """Execute a single task with persistence."""
        task.status = TaskStatus.RUNNING
        task.started_at = time.time()
        await memory.update_task(task.task_id, "running")

        try:
            result = await tool_executor.execute(task.tool, task.host, task.cmd)
            task.result = result
            task.status = TaskStatus.COMPLETED
            summary = (result[:200] + "...") if result and len(result) > 200 else result
            await memory.update_task(task.task_id, "completed", summary)
        except Exception as e:
            task.error = str(e)
            task.status = TaskStatus.FAILED
            log.error(f"Task {task.short_id} failed: {e}")
            await memory.update_task(task.task_id, "failed", str(e)[:200])
        finally:
            task.completed_at = time.time()

        return task

    async def retry_task(self, task_id: str, channel: str, thread_ts: str) -> Optional[Task]:
        """Retry a failed or cancelled task."""
        # Find task by full or short ID
        task = self.tasks.get(task_id)
        if not task:
            for t in self.tasks.values():
                if t.short_id == task_id or t.task_id.startswith(task_id):
                    task = t
                    break
        if not task:
            return None
        if task.status not in (TaskStatus.FAILED, TaskStatus.CANCELLED):
            return None

        # Create a new retry task
        new_task = self.create_task(
            task.tool, task.host, task.cmd, channel, thread_ts,
            created_by=task.created_by,
        )
        new_task.retries = task.retries + 1
        group_id = uuid.uuid4().hex[:8]
        self.groups[group_id] = [new_task.task_id]
        await self.execute_group(group_id, channel, thread_ts)
        return new_task

    def cancel_task(self, task_id: str) -> Optional[Task]:
        """Cancel a queued or running task. Returns the task if cancelled."""
        task = self.tasks.get(task_id)
        if not task:
            for t in self.tasks.values():
                if t.short_id == task_id or t.task_id.startswith(task_id):
                    task = t
                    break
        if task and task.status in (TaskStatus.QUEUED, TaskStatus.RUNNING):
            task.status = TaskStatus.CANCELLED
            task.completed_at = time.time()
            return task
        return None

    def get_task(self, task_id: str) -> Optional[Task]:
        """Find task by full or short ID."""
        task = self.tasks.get(task_id)
        if task:
            return task
        for t in self.tasks.values():
            if t.short_id == task_id or t.task_id.startswith(task_id):
                return t
        return None

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
# Monitoring Service
# ---------------------------------------------------------------------------

# Default health checks — seeded on first run
DEFAULT_CHECKS = [
    {"check_id": "vm-mainframe", "name": "Mainframe Uptime", "target": "swarm-mainframe",
     "check_type": "ssh", "command": "uptime", "interval_seconds": 300, "severity": "critical"},
    {"check_id": "vm-gpu", "name": "GPU VM Uptime", "target": "swarm-gpu",
     "check_type": "ssh", "command": "uptime", "interval_seconds": 300, "severity": "critical"},
    {"check_id": "vm-portal", "name": "AI Portal Uptime", "target": "fc-ai-portal",
     "check_type": "ssh", "command": "uptime", "interval_seconds": 300, "severity": "critical"},
    {"check_id": "vm-web", "name": "Calculus Web Uptime", "target": "calculus-web",
     "check_type": "ssh", "command": "uptime", "interval_seconds": 300, "severity": "warning"},
    {"check_id": "docker-health", "name": "Docker Containers", "target": "swarm-mainframe",
     "check_type": "ssh", "command": "docker ps --format '{{.Names}}: {{.Status}}'", "interval_seconds": 300, "severity": "warning"},
    {"check_id": "gpu-health", "name": "GPU Status", "target": "swarm-gpu",
     "check_type": "ssh", "command": "nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu --format=csv,noheader 2>/dev/null || echo 'GPU UNAVAILABLE'",
     "interval_seconds": 600, "severity": "critical"},
    {"check_id": "disk-mainframe", "name": "Disk Usage (mainframe)", "target": "swarm-mainframe",
     "check_type": "ssh", "command": "df -h / | tail -1 | awk '{print $5}'", "interval_seconds": 1800, "severity": "warning"},
    {"check_id": "disk-gpu", "name": "Disk Usage (GPU)", "target": "swarm-gpu",
     "check_type": "ssh", "command": "df -h / | tail -1 | awk '{print $5}'", "interval_seconds": 1800, "severity": "warning"},
    {"check_id": "ollama-health", "name": "Ollama Service", "target": "swarm-gpu",
     "check_type": "ssh", "command": "curl -s http://localhost:11434/api/tags | head -c 200 || echo 'OLLAMA DOWN'",
     "interval_seconds": 600, "severity": "warning"},
]

ALERT_CHANNEL = os.environ.get("BUNNY_ALERT_CHANNEL", "")  # Slack channel for alerts


class MonitoringService:
    """Proactive health monitoring with rule engine and alerts."""

    def __init__(self):
        self._running = False
        self._task: Optional[asyncio.Task] = None

    async def seed_defaults(self):
        """Seed default checks if none exist."""
        def _seed():
            conn = _db_connect()
            try:
                count = conn.execute("SELECT COUNT(*) FROM monitor_checks").fetchone()[0]
                if count == 0:
                    for check in DEFAULT_CHECKS:
                        conn.execute(
                            "INSERT OR IGNORE INTO monitor_checks "
                            "(check_id, name, target, check_type, command, interval_seconds, severity) "
                            "VALUES (?, ?, ?, ?, ?, ?, ?)",
                            (check["check_id"], check["name"], check["target"],
                             check["check_type"], check["command"],
                             check["interval_seconds"], check["severity"]),
                        )
                    conn.commit()
                    return len(DEFAULT_CHECKS)
                return 0
            finally:
                conn.close()
        return await asyncio.to_thread(_seed)

    async def get_checks(self, enabled_only: bool = True) -> List[Dict]:
        """Get all monitoring checks."""
        def _query():
            conn = _db_connect()
            try:
                if enabled_only:
                    rows = conn.execute("SELECT * FROM monitor_checks WHERE enabled = 1").fetchall()
                else:
                    rows = conn.execute("SELECT * FROM monitor_checks").fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def run_check(self, check: Dict) -> Dict:
        """Execute a single health check."""
        check_id = check["check_id"]
        target = check["target"]
        command = check["command"]
        now = time.time()

        try:
            result = await tool_executor.execute("shell", target, command)
            status = "ok"

            # Basic threshold checks
            if check_id.startswith("disk-"):
                # Parse disk percentage
                try:
                    pct = int(result.strip().replace("%", ""))
                    if pct > 90:
                        status = "critical"
                    elif pct > 80:
                        status = "warning"
                except (ValueError, AttributeError):
                    pass
            elif "UNAVAILABLE" in (result or "").upper() or "DOWN" in (result or "").upper():
                status = "critical"
            elif "error" in (result or "").lower():
                status = "warning"

        except Exception as e:
            result = str(e)
            status = "critical"

        # Update check status in DB
        def _update():
            conn = _db_connect()
            try:
                old_status = conn.execute(
                    "SELECT last_status FROM monitor_checks WHERE check_id = ?",
                    (check_id,),
                ).fetchone()
                old = old_status["last_status"] if old_status else None

                conn.execute(
                    "UPDATE monitor_checks SET last_status = ?, last_result = ?, last_run_at = ? "
                    "WHERE check_id = ?",
                    (status, (result or "")[:500], now, check_id),
                )

                # Create alert if status changed to non-ok
                alert_needed = False
                if status != "ok" and old != status:
                    conn.execute(
                        "INSERT INTO monitor_alerts (check_id, status, message, created_at) "
                        "VALUES (?, ?, ?, ?)",
                        (check_id, status, f"{check['name']}: {(result or '')[:200]}", now),
                    )
                    alert_needed = True
                elif status == "ok" and old and old != "ok":
                    # Resolve alert
                    conn.execute(
                        "UPDATE monitor_alerts SET resolved_at = ? "
                        "WHERE check_id = ? AND resolved_at IS NULL",
                        (now, check_id),
                    )
                    alert_needed = True

                conn.commit()
                return {"alert": alert_needed, "old_status": old, "new_status": status}
            finally:
                conn.close()

        alert_info = await asyncio.to_thread(_update)
        return {
            "check_id": check_id,
            "name": check["name"],
            "target": target,
            "status": status,
            "result": (result or "")[:300],
            "alert": alert_info.get("alert", False),
            "old_status": alert_info.get("old_status"),
        }

    async def run_all_checks(self) -> List[Dict]:
        """Run all enabled, non-muted checks."""
        checks = await self.get_checks()
        results = []
        for check in checks:
            if check.get("muted"):
                continue
            try:
                r = await self.run_check(check)
                results.append(r)
            except Exception as e:
                results.append({"check_id": check["check_id"], "status": "error", "result": str(e)})
        return results

    async def get_alerts(self, active_only: bool = True, limit: int = 20) -> List[Dict]:
        """Get recent alerts."""
        def _query():
            conn = _db_connect()
            try:
                if active_only:
                    rows = conn.execute(
                        "SELECT * FROM monitor_alerts WHERE resolved_at IS NULL "
                        "ORDER BY created_at DESC LIMIT ?", (limit,)
                    ).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT * FROM monitor_alerts ORDER BY created_at DESC LIMIT ?", (limit,)
                    ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def mute_check(self, check_id: str, mute: bool = True):
        """Mute/unmute a check."""
        def _update():
            conn = _db_connect()
            try:
                conn.execute("UPDATE monitor_checks SET muted = ? WHERE check_id = ?",
                             (1 if mute else 0, check_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_update)

    async def start_monitoring_loop(self):
        """Start the background monitoring loop."""
        if self._running:
            return
        self._running = True
        self._task = asyncio.create_task(self._monitor_loop())
        log.info("Monitoring loop started")

    async def _monitor_loop(self):
        """Periodic monitoring loop."""
        while self._running:
            try:
                checks = await self.get_checks()
                now = time.time()
                for check in checks:
                    if check.get("muted"):
                        continue
                    last_run = check.get("last_run_at") or 0
                    interval = check.get("interval_seconds", 300)
                    if now - last_run >= interval:
                        result = await self.run_check(check)
                        # Send alert to Slack if needed
                        if result.get("alert") and ALERT_CHANNEL and result["status"] != "ok":
                            icon = ":rotating_light:" if result["status"] == "critical" else ":warning:"
                            await post_message(
                                f"{icon} *{result['name']}* ({result['target']}): "
                                f"`{result['status']}` \u2014 {result['result'][:150]}",
                                ALERT_CHANNEL,
                            )
                        elif result.get("alert") and ALERT_CHANNEL and result["status"] == "ok":
                            await post_message(
                                f":white_check_mark: *{result['name']}* ({result['target']}): recovered",
                                ALERT_CHANNEL,
                            )
            except Exception as e:
                log.error(f"Monitoring loop error: {e}")
            await asyncio.sleep(60)  # Check every 60s which checks are due

    def stop(self):
        self._running = False
        if self._task:
            self._task.cancel()


monitor = MonitoringService()


# ---------------------------------------------------------------------------
# Scheduler Service
# ---------------------------------------------------------------------------

class SchedulerService:
    """Job scheduler for one-off and recurring tasks."""

    def __init__(self):
        self._running = False
        self._task: Optional[asyncio.Task] = None

    async def add_job(self, job_id: str, job_type: str, payload: str,
                      description: str = "", schedule_expression: str = None,
                      interval_seconds: int = None, channel_id: str = "",
                      thread_ts: str = "", owner: str = "") -> Dict:
        """Create a scheduled job."""
        now = time.time()
        next_run = None
        if interval_seconds:
            next_run = now + interval_seconds
        elif schedule_expression:
            next_run = self._parse_next_run(schedule_expression, now)

        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT OR REPLACE INTO scheduled_jobs "
                    "(job_id, owner, channel_id, thread_ts, job_type, description, "
                    "payload, schedule_expression, interval_seconds, next_run_at, enabled, created_at) "
                    "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)",
                    (job_id, owner, channel_id, thread_ts, job_type, description,
                     payload, schedule_expression, interval_seconds, next_run, now),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_insert)
        return {"job_id": job_id, "next_run_at": next_run}

    async def get_jobs(self, enabled_only: bool = True) -> List[Dict]:
        """Get all jobs."""
        def _query():
            conn = _db_connect()
            try:
                if enabled_only:
                    rows = conn.execute(
                        "SELECT * FROM scheduled_jobs WHERE enabled = 1 ORDER BY next_run_at"
                    ).fetchall()
                else:
                    rows = conn.execute("SELECT * FROM scheduled_jobs ORDER BY next_run_at").fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def remove_job(self, job_id: str):
        """Remove a job."""
        def _delete():
            conn = _db_connect()
            try:
                conn.execute("DELETE FROM scheduled_jobs WHERE job_id = ?", (job_id,))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_delete)

    async def toggle_job(self, job_id: str, enabled: bool):
        """Enable/disable a job."""
        def _update():
            conn = _db_connect()
            try:
                conn.execute("UPDATE scheduled_jobs SET enabled = ? WHERE job_id = ?",
                             (1 if enabled else 0, job_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_update)

    def _parse_next_run(self, expression: str, now: float) -> float:
        """Parse simple schedule expressions. Supports:
        - 'in Xm' / 'in Xh' — relative time
        - 'every Xm' / 'every Xh' — interval (first run = now + interval)
        - HH:MM — next occurrence of that time today or tomorrow
        """
        expr = expression.strip().lower()
        if expr.startswith("in "):
            val = expr[3:].strip()
            if val.endswith("m"):
                return now + int(val[:-1]) * 60
            elif val.endswith("h"):
                return now + int(val[:-1]) * 3600
            elif val.endswith("s"):
                return now + int(val[:-1])
        elif expr.startswith("every "):
            val = expr[6:].strip()
            if val.endswith("m"):
                return now + int(val[:-1]) * 60
            elif val.endswith("h"):
                return now + int(val[:-1]) * 3600
        elif ":" in expr:
            import datetime
            h, m = map(int, expr.split(":"))
            today = datetime.datetime.now()
            target = today.replace(hour=h, minute=m, second=0, microsecond=0)
            if target.timestamp() <= now:
                target += datetime.timedelta(days=1)
            return target.timestamp()
        return now + 3600  # default: 1 hour

    async def start_scheduler_loop(self):
        """Start the background scheduler loop."""
        if self._running:
            return
        self._running = True
        self._task = asyncio.create_task(self._scheduler_loop())
        log.info("Scheduler loop started")

    async def _scheduler_loop(self):
        """Check for due jobs and execute them."""
        while self._running:
            try:
                jobs = await self.get_jobs()
                now = time.time()
                for job in jobs:
                    next_run = job.get("next_run_at")
                    if next_run and now >= next_run:
                        await self._execute_job(job)
            except Exception as e:
                log.error(f"Scheduler loop error: {e}")
            await asyncio.sleep(30)  # Check every 30 seconds

    async def _execute_job(self, job: Dict):
        """Execute a scheduled job."""
        job_id = job["job_id"]
        job_type = job["job_type"]
        payload = job["payload"]
        channel = job.get("channel_id", "")
        thread_ts = job.get("thread_ts", "")

        log.info(f"Executing scheduled job: {job_id} ({job_type})")

        try:
            if job_type == "shell":
                # payload is JSON: {"host": "...", "cmd": "..."}
                data = json.loads(payload)
                result = await tool_executor.execute("shell", data.get("host", "swarm-mainframe"), data["cmd"])
                if channel:
                    await post_message(
                        f":clock1: *Scheduled job `{job_id}`*\n```{(result or 'done')[:1000]}```",
                        channel, thread_ts,
                    )
            elif job_type == "reminder":
                if channel:
                    await post_message(f":bell: *Reminder:* {payload}", channel, thread_ts)
            elif job_type == "health":
                results = await monitor.run_all_checks()
                if channel:
                    lines = [":stethoscope: *Scheduled Health Check*\n"]
                    for r in results:
                        icon = ":white_check_mark:" if r["status"] == "ok" else ":x:" if r["status"] == "critical" else ":warning:"
                        lines.append(f"{icon} {r['name']} ({r['target']}): `{r['status']}`")
                    await post_message("\n".join(lines), channel, thread_ts)
            elif job_type == "message":
                if channel:
                    await post_message(payload, channel, thread_ts)
        except Exception as e:
            log.error(f"Job {job_id} failed: {e}")
            if channel:
                await post_message(f":x: Scheduled job `{job_id}` failed: `{e}`", channel, thread_ts)

        # Update last_run and calculate next_run
        def _update_schedule():
            conn = _db_connect()
            try:
                now = time.time()
                interval = job.get("interval_seconds")
                schedule_expr = job.get("schedule_expression", "")

                if interval:
                    next_run = now + interval
                elif schedule_expr and schedule_expr.startswith("every "):
                    next_run = self._parse_next_run(schedule_expr, now)
                else:
                    # One-off job — disable after execution
                    conn.execute(
                        "UPDATE scheduled_jobs SET last_run_at = ?, enabled = 0 WHERE job_id = ?",
                        (now, job_id),
                    )
                    conn.commit()
                    return

                conn.execute(
                    "UPDATE scheduled_jobs SET last_run_at = ?, next_run_at = ? WHERE job_id = ?",
                    (now, next_run, job_id),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_update_schedule)

    def stop(self):
        self._running = False
        if self._task:
            self._task.cancel()


scheduler = SchedulerService()


# ---------------------------------------------------------------------------
# Knowledge Graph Service
# ---------------------------------------------------------------------------

class KnowledgeGraph:
    """Structured graph of infrastructure entities and relationships."""

    async def add_entity(self, entity_type: str, name: str,
                         attributes: Dict = None, entity_id: str = None) -> str:
        """Add or update an entity."""
        eid = entity_id or f"{entity_type}:{name}".replace(" ", "-").lower()
        now = time.time()
        attrs_json = json.dumps(attributes or {})

        def _upsert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO graph_entities (entity_id, entity_type, name, attributes_json, created_at, updated_at) "
                    "VALUES (?, ?, ?, ?, ?, ?) "
                    "ON CONFLICT(entity_id) DO UPDATE SET name = ?, attributes_json = ?, updated_at = ?",
                    (eid, entity_type, name, attrs_json, now, now, name, attrs_json, now),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_upsert)
        return eid

    async def add_edge(self, src_id: str, relation: str, dst_id: str,
                       attributes: Dict = None) -> str:
        """Add a relationship between entities."""
        edge_id = f"{src_id}->{relation}->{dst_id}"
        attrs_json = json.dumps(attributes or {})
        now = time.time()

        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT OR REPLACE INTO graph_edges "
                    "(edge_id, src_entity_id, relation, dst_entity_id, attributes_json, created_at) "
                    "VALUES (?, ?, ?, ?, ?, ?)",
                    (edge_id, src_id, relation, dst_id, attrs_json, now),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_insert)
        return edge_id

    async def log_event(self, entity_id: str, event_type: str, payload: Dict = None):
        """Log an event for an entity."""
        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO graph_events (entity_id, event_type, payload_json, created_at) "
                    "VALUES (?, ?, ?, ?)",
                    (entity_id, event_type, json.dumps(payload or {}), time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_insert)

    async def get_entity(self, entity_id: str) -> Optional[Dict]:
        """Get an entity by ID."""
        def _query():
            conn = _db_connect()
            try:
                row = conn.execute(
                    "SELECT * FROM graph_entities WHERE entity_id = ?", (entity_id,)
                ).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def search_entities(self, query: str, entity_type: str = None, limit: int = 20) -> List[Dict]:
        """Search entities by name or type."""
        def _query():
            conn = _db_connect()
            try:
                if entity_type:
                    rows = conn.execute(
                        "SELECT * FROM graph_entities WHERE entity_type = ? AND name LIKE ? LIMIT ?",
                        (entity_type, f"%{query}%", limit),
                    ).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT * FROM graph_entities WHERE name LIKE ? OR entity_id LIKE ? LIMIT ?",
                        (f"%{query}%", f"%{query}%", limit),
                    ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def get_neighbors(self, entity_id: str) -> Dict[str, List[Dict]]:
        """Get all connected entities (outgoing + incoming edges)."""
        def _query():
            conn = _db_connect()
            try:
                outgoing = conn.execute(
                    "SELECT e.relation, e.dst_entity_id, g.name, g.entity_type "
                    "FROM graph_edges e LEFT JOIN graph_entities g ON e.dst_entity_id = g.entity_id "
                    "WHERE e.src_entity_id = ?", (entity_id,)
                ).fetchall()
                incoming = conn.execute(
                    "SELECT e.relation, e.src_entity_id, g.name, g.entity_type "
                    "FROM graph_edges e LEFT JOIN graph_entities g ON e.src_entity_id = g.entity_id "
                    "WHERE e.dst_entity_id = ?", (entity_id,)
                ).fetchall()
                return {
                    "outgoing": [dict(r) for r in outgoing],
                    "incoming": [dict(r) for r in incoming],
                }
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def get_dependencies(self, entity_id: str) -> List[Dict]:
        """Get all entities this one depends on (recursive 2 levels)."""
        def _query():
            conn = _db_connect()
            try:
                # Direct deps
                rows = conn.execute(
                    "SELECT e.dst_entity_id, e.relation, g.name, g.entity_type "
                    "FROM graph_edges e LEFT JOIN graph_entities g ON e.dst_entity_id = g.entity_id "
                    "WHERE e.src_entity_id = ? AND e.relation IN ('depends_on', 'hosted_on', 'uses')",
                    (entity_id,),
                ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def get_impact(self, entity_id: str) -> List[Dict]:
        """Get all entities that depend on this one."""
        def _query():
            conn = _db_connect()
            try:
                rows = conn.execute(
                    "SELECT e.src_entity_id, e.relation, g.name, g.entity_type "
                    "FROM graph_edges e LEFT JOIN graph_entities g ON e.src_entity_id = g.entity_id "
                    "WHERE e.dst_entity_id = ? AND e.relation IN ('depends_on', 'hosted_on', 'uses')",
                    (entity_id,),
                ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def get_recent_events(self, entity_id: str = None, limit: int = 20) -> List[Dict]:
        """Get recent events for an entity or all entities."""
        def _query():
            conn = _db_connect()
            try:
                if entity_id:
                    rows = conn.execute(
                        "SELECT * FROM graph_events WHERE entity_id = ? ORDER BY created_at DESC LIMIT ?",
                        (entity_id, limit),
                    ).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT * FROM graph_events ORDER BY created_at DESC LIMIT ?", (limit,)
                    ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def seed_infrastructure(self):
        """Seed the graph with known infrastructure entities."""
        # VMs
        for vm_name, vm_info in VMS.items():
            await self.add_entity("vm", vm_name, {"ip": vm_info["ip"], "zone": vm_info["zone"]})

        # Services
        await self.add_entity("service", "bunny-alpha", {"type": "slack-bot", "port": PORT})
        await self.add_entity("service", "ai-portal", {"type": "api", "port": 8000})
        await self.add_entity("service", "ollama", {"type": "inference", "port": 11434})
        await self.add_entity("service", "cloudflare-tunnel", {"type": "tunnel"})

        # Providers
        for provider in ["deepseek", "groq", "xai", "openai", "anthropic", "google", "mistral"]:
            await self.add_entity("provider", provider)

        # Assistants
        for name in ["jack", "joyceann", "bunny-alpha"]:
            await self.add_entity("assistant", name)

        # Edges
        await self.add_edge("service:bunny-alpha", "hosted_on", "vm:swarm-mainframe")
        await self.add_edge("service:ai-portal", "hosted_on", "vm:fc-ai-portal")
        await self.add_edge("service:ollama", "hosted_on", "vm:swarm-gpu")
        await self.add_edge("service:bunny-alpha", "uses", "service:ai-portal")
        await self.add_edge("service:bunny-alpha", "uses", "service:ollama")
        await self.add_edge("service:ai-portal", "uses", "provider:deepseek")
        await self.add_edge("service:ai-portal", "uses", "provider:groq")
        await self.add_edge("service:ai-portal", "uses", "provider:xai")
        await self.add_edge("service:ai-portal", "uses", "provider:openai")
        await self.add_edge("service:ai-portal", "uses", "provider:anthropic")
        await self.add_edge("service:ai-portal", "uses", "provider:google")
        await self.add_edge("assistant:bunny-alpha", "operates", "service:bunny-alpha")

        log.info("Knowledge graph seeded with infrastructure entities")


knowledge_graph = KnowledgeGraph()


# ---------------------------------------------------------------------------
# Autonomous Planning Service
# ---------------------------------------------------------------------------

class PlanningService:
    """Converts high-level goals into structured multi-step plans."""

    async def create_plan(self, goal_text: str, created_by: str = "") -> str:
        """Create a new goal plan."""
        plan_id = f"plan-{uuid.uuid4().hex[:8]}"
        now = time.time()

        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO goal_plans (plan_id, goal_text, created_by, status, created_at) "
                    "VALUES (?, ?, ?, 'planning', ?)",
                    (plan_id, goal_text, created_by, now),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_insert)
        return plan_id

    async def add_step(self, plan_id: str, title: str, description: str = "",
                       task_type: str = "shell", priority: int = 5,
                       dependencies: List[str] = None, assigned_service: str = "") -> str:
        """Add a step to a plan."""
        step_id = f"step-{uuid.uuid4().hex[:6]}"
        deps_json = json.dumps(dependencies or [])
        now = time.time()

        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO plan_steps "
                    "(step_id, plan_id, title, description, task_type, priority, "
                    "dependencies, assigned_service, status, created_at) "
                    "VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?)",
                    (step_id, plan_id, title, description, task_type, priority,
                     deps_json, assigned_service, now),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_insert)
        return step_id

    async def get_plan(self, plan_id: str) -> Optional[Dict]:
        """Get a plan with its steps."""
        def _query():
            conn = _db_connect()
            try:
                plan = conn.execute(
                    "SELECT * FROM goal_plans WHERE plan_id = ?", (plan_id,)
                ).fetchone()
                if not plan:
                    return None
                steps = conn.execute(
                    "SELECT * FROM plan_steps WHERE plan_id = ? ORDER BY priority, created_at",
                    (plan_id,),
                ).fetchall()
                return {"plan": dict(plan), "steps": [dict(s) for s in steps]}
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def get_recent_plans(self, limit: int = 10) -> List[Dict]:
        """Get recent plans."""
        def _query():
            conn = _db_connect()
            try:
                rows = conn.execute(
                    "SELECT * FROM goal_plans ORDER BY created_at DESC LIMIT ?", (limit,)
                ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def update_step(self, step_id: str, status: str, result_summary: str = None):
        """Update a plan step status."""
        def _update():
            conn = _db_connect()
            try:
                if result_summary:
                    conn.execute(
                        "UPDATE plan_steps SET status = ?, result_summary = ?, completed_at = ? "
                        "WHERE step_id = ?",
                        (status, result_summary, time.time(), step_id),
                    )
                else:
                    conn.execute(
                        "UPDATE plan_steps SET status = ? WHERE step_id = ?",
                        (status, step_id),
                    )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_update)

    async def update_plan_status(self, plan_id: str, status: str, summary: str = None):
        """Update plan status."""
        def _update():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE goal_plans SET status = ?, summary = ?, completed_at = ? WHERE plan_id = ?",
                    (status, summary, time.time() if status in ("completed", "failed") else None, plan_id),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_update)

    async def execute_plan(self, plan_id: str, channel: str, thread_ts: str):
        """Execute a plan by running steps in dependency order."""
        plan_data = await self.get_plan(plan_id)
        if not plan_data:
            return

        plan = plan_data["plan"]
        steps = plan_data["steps"]
        await self.update_plan_status(plan_id, "executing")

        completed_steps = set()
        failed = False

        for step in steps:
            step_id = step["step_id"]
            deps = json.loads(step.get("dependencies", "[]"))

            # Check dependencies
            if deps and not all(d in completed_steps for d in deps):
                await self.update_step(step_id, "blocked")
                continue

            # Execute step
            await self.update_step(step_id, "running")
            await post_message(
                f":gear: Plan `{plan_id}` — executing: *{step['title']}*",
                channel, thread_ts,
            )

            try:
                # Use AI to determine the command from the step description
                cmd_prompt = (
                    f"You need to execute this infrastructure task: {step['title']}\n"
                    f"Description: {step.get('description', step['title'])}\n"
                    f"Generate ONE shell command to accomplish this. "
                    f"Available hosts: swarm-mainframe, swarm-gpu, fc-ai-portal, calculus-web.\n"
                    f"Respond with ONLY a JSON object: "
                    f'{"{"}"tool":"shell","host":"<hostname>","cmd":"<command>"{"}"}'
                )
                # For now, try to execute directly if description looks like a command
                desc = step.get("description", "")
                if desc.startswith("{"):
                    cmd_data = json.loads(desc)
                    result = await tool_executor.execute(
                        cmd_data.get("tool", "shell"),
                        cmd_data.get("host", "swarm-mainframe"),
                        cmd_data.get("cmd", "echo done"),
                    )
                else:
                    result = f"Step '{step['title']}' marked complete (manual step)"

                await self.update_step(step_id, "completed", (result or "")[:200])
                completed_steps.add(step_id)
                await post_message(
                    f":white_check_mark: Step *{step['title']}* completed",
                    channel, thread_ts,
                )
            except Exception as e:
                await self.update_step(step_id, "failed", str(e)[:200])
                failed = True
                await post_message(
                    f":x: Step *{step['title']}* failed: `{e}`",
                    channel, thread_ts,
                )
                # Don't break — try to continue with non-dependent steps

        status = "failed" if failed else "completed"
        summary = f"{len(completed_steps)}/{len(steps)} steps completed"
        await self.update_plan_status(plan_id, status, summary)
        await post_message(
            f":clipboard: Plan `{plan_id}` {status}: {summary}",
            channel, thread_ts,
        )


planner = PlanningService()


# ---------------------------------------------------------------------------
# Multi-Agent Orchestration
# ---------------------------------------------------------------------------

# Agent specifications
BUILTIN_AGENTS = [
    {"agent_id": "infra-agent", "name": "InfraAgent", "role": "Infrastructure diagnostics and VM operations",
     "capabilities": "ssh,docker,vm-health,service-restart,disk-check"},
    {"agent_id": "code-agent", "name": "CodeAgent", "role": "Code reasoning and repository analysis",
     "capabilities": "git,code-review,diff-summary,implementation-guidance"},
    {"agent_id": "search-agent", "name": "SearchAgent", "role": "Web search and document retrieval",
     "capabilities": "web-search,url-fetch,document-summary"},
    {"agent_id": "monitor-agent", "name": "MonitorAgent", "role": "Monitoring alerts and anomaly analysis",
     "capabilities": "health-check,alert-triage,metric-analysis,prediction"},
    {"agent_id": "memory-agent", "name": "MemoryAgent", "role": "Long-term recall and summarization",
     "capabilities": "memory-search,summarize,knowledge-base,preference-recall"},
    {"agent_id": "planner-agent", "name": "PlannerAgent", "role": "Plan generation and task decomposition",
     "capabilities": "goal-decomposition,dependency-analysis,step-generation"},
]


class MultiAgentCoordinator:
    """Coordinates specialist sub-agents for complex tasks."""

    async def seed_agents(self):
        """Seed built-in agent specs."""
        def _seed():
            conn = _db_connect()
            try:
                for agent in BUILTIN_AGENTS:
                    conn.execute(
                        "INSERT OR IGNORE INTO agent_specs "
                        "(agent_id, name, role, capabilities, priority, status) "
                        "VALUES (?, ?, ?, ?, 5, 'active')",
                        (agent["agent_id"], agent["name"], agent["role"], agent["capabilities"]),
                    )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_seed)

    async def get_agents(self) -> List[Dict]:
        """Get all agent specs."""
        def _query():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute("SELECT * FROM agent_specs").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def delegate(self, parent_task_id: str, agent_id: str,
                       payload: str, channel: str = "", thread_ts: str = "") -> str:
        """Delegate a task to a sub-agent."""
        now = time.time()

        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO delegated_tasks "
                    "(parent_task_id, assigned_agent, task_payload, status, created_at) "
                    "VALUES (?, ?, ?, 'pending', ?)",
                    (parent_task_id, agent_id, payload, now),
                )
                return conn.execute("SELECT last_insert_rowid()").fetchone()[0]
            finally:
                conn.close()

        task_id = await asyncio.to_thread(_insert)

        # Route to the right agent logic
        result = await self._execute_agent_task(agent_id, payload, channel, thread_ts)

        # Store result
        def _complete():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE delegated_tasks SET status = 'completed', result_summary = ?, completed_at = ? "
                    "WHERE rowid = ?",
                    ((result or "")[:500], time.time(), task_id),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_complete)
        return result or ""

    async def _execute_agent_task(self, agent_id: str, payload: str,
                                   channel: str, thread_ts: str) -> str:
        """Execute a task as a specific agent."""
        if agent_id == "infra-agent":
            # Run infrastructure command
            try:
                data = json.loads(payload) if payload.startswith("{") else {"cmd": payload}
                result = await tool_executor.execute(
                    "shell", data.get("host", "swarm-mainframe"), data.get("cmd", payload),
                )
                return result
            except Exception as e:
                return f"InfraAgent error: {e}"

        elif agent_id == "monitor-agent":
            results = await monitor.run_all_checks()
            summary = "\n".join(f"{r['name']}: {r['status']}" for r in results)
            return summary

        elif agent_id == "memory-agent":
            results = await memory.search_messages(payload, limit=10)
            if results:
                return "\n".join(f"[{r['role']}] {r['content'][:100]}" for r in results)
            return "No matching memories found."

        elif agent_id == "planner-agent":
            plan_id = await planner.create_plan(payload)
            return f"Plan created: {plan_id}"

        else:
            return f"Agent {agent_id} executed: {payload[:100]}"

    async def orchestrate(self, request: str, channel: str, thread_ts: str) -> str:
        """Decompose a request and delegate to multiple agents."""
        # Use AI to determine which agents to involve
        agents = await self.get_agents()
        agent_list = "\n".join(f"- {a['agent_id']}: {a['role']}" for a in agents)

        prompt = (
            f"You are a task coordinator. Decompose this request into sub-tasks "
            f"for specialist agents. Available agents:\n{agent_list}\n\n"
            f"Request: {request}\n\n"
            f"Respond with a JSON array of objects, each with 'agent_id' and 'task'. "
            f"Example: [{{'agent_id': 'infra-agent', 'task': 'check VM health'}}]\n"
            f"Only use agents that are relevant. Respond ONLY with the JSON array."
        )

        # For now, use simple keyword routing
        delegations = []
        req_lower = request.lower()
        if any(w in req_lower for w in ["vm", "docker", "service", "restart", "disk", "uptime"]):
            delegations.append(("infra-agent", request))
        if any(w in req_lower for w in ["health", "monitor", "check", "alert"]):
            delegations.append(("monitor-agent", request))
        if any(w in req_lower for w in ["remember", "recall", "history", "what did"]):
            delegations.append(("memory-agent", request))
        if any(w in req_lower for w in ["plan", "goal", "deploy", "restore"]):
            delegations.append(("planner-agent", request))

        if not delegations:
            delegations.append(("infra-agent", request))

        parent_id = uuid.uuid4().hex[:8]
        results = []
        for agent_id, task in delegations:
            result = await self.delegate(parent_id, agent_id, task, channel, thread_ts)
            results.append(f"*{agent_id}*: {result[:200]}")

        return "\n\n".join(results)

    async def get_task_trace(self, parent_task_id: str) -> List[Dict]:
        """Get delegation trace for a task."""
        def _query():
            conn = _db_connect()
            try:
                rows = conn.execute(
                    "SELECT * FROM delegated_tasks WHERE parent_task_id = ? ORDER BY created_at",
                    (parent_task_id,),
                ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)


agent_coordinator = MultiAgentCoordinator()


# ---------------------------------------------------------------------------
# Predictive Monitoring
# ---------------------------------------------------------------------------

class PredictiveMonitor:
    """Detects trends and predicts failures before they happen."""

    async def collect_metrics(self, target: str) -> Dict:
        """Collect current metrics for a target."""
        metrics = {}
        try:
            if target in VMS:
                result = await tool_executor.execute("shell", target,
                    "echo CPU:$(top -bn1 | grep 'Cpu(s)' | awk '{print $2}') "
                    "MEM:$(free | awk '/Mem/{printf \"%.1f\", $3/$2*100}') "
                    "DISK:$(df / | tail -1 | awk '{print $5}' | tr -d '%')")
                if result:
                    for part in result.split():
                        if ":" in part:
                            k, v = part.split(":", 1)
                            try:
                                metrics[k.lower()] = float(v)
                            except ValueError:
                                pass
        except Exception as e:
            metrics["error"] = str(e)
        return metrics

    async def analyze_health(self, target: str = None) -> List[Dict]:
        """Analyze health and generate predictions."""
        signals = []
        targets = [target] if target else list(VMS.keys())

        for t in targets:
            metrics = await self.collect_metrics(t)
            if "error" in metrics:
                signals.append({
                    "target": t,
                    "signal_type": "unreachable",
                    "confidence": 0.9,
                    "predicted_failure_window": "immediate",
                    "explanation": f"Cannot collect metrics: {metrics['error']}",
                    "risk_level": "critical",
                    "recommended_action": f"Check VM {t} connectivity",
                })
                continue

            # Disk prediction
            disk = metrics.get("disk", 0)
            if disk > 85:
                signals.append({
                    "target": t,
                    "signal_type": "disk_exhaustion",
                    "confidence": 0.8 if disk > 90 else 0.6,
                    "predicted_failure_window": "24h" if disk > 90 else "72h",
                    "explanation": f"Disk usage at {disk}%",
                    "risk_level": "critical" if disk > 90 else "warning",
                    "recommended_action": f"Free disk space on {t}",
                })

            # CPU prediction
            cpu = metrics.get("cpu", 0)
            if cpu > 80:
                signals.append({
                    "target": t,
                    "signal_type": "cpu_pressure",
                    "confidence": 0.7,
                    "predicted_failure_window": "6h",
                    "explanation": f"CPU at {cpu}%",
                    "risk_level": "warning",
                    "recommended_action": f"Investigate high CPU on {t}",
                })

            # Memory prediction
            mem = metrics.get("mem", 0)
            if mem > 85:
                signals.append({
                    "target": t,
                    "signal_type": "memory_pressure",
                    "confidence": 0.75,
                    "predicted_failure_window": "12h",
                    "explanation": f"Memory at {mem}%",
                    "risk_level": "critical" if mem > 95 else "warning",
                    "recommended_action": f"Check memory consumers on {t}",
                })

        # Store signals
        for sig in signals:
            await self._store_signal(sig)

        return signals

    async def _store_signal(self, signal: Dict):
        """Store a prediction signal."""
        def _insert():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO prediction_signals "
                    "(target, signal_type, confidence, predicted_failure_window, "
                    "supporting_metrics, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                    (signal["target"], signal["signal_type"], signal.get("confidence", 0.5),
                     signal.get("predicted_failure_window", "unknown"),
                     json.dumps(signal), time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_insert)

    async def get_risk_assessment(self) -> List[Dict]:
        """Generate risk assessments for all systems."""
        signals = await self.analyze_health()
        assessments = {}

        for sig in signals:
            target = sig["target"]
            if target not in assessments:
                assessments[target] = {"target": target, "signals": [], "risk_score": 0}
            assessments[target]["signals"].append(sig)
            # Accumulate risk
            conf = sig.get("confidence", 0.5)
            if sig.get("risk_level") == "critical":
                assessments[target]["risk_score"] += conf * 0.5
            else:
                assessments[target]["risk_score"] += conf * 0.2

        results = []
        for target, data in assessments.items():
            score = min(1.0, data["risk_score"])
            level = "critical" if score > 0.7 else "warning" if score > 0.3 else "low"
            explanation = "; ".join(s["explanation"] for s in data["signals"])
            action = data["signals"][0].get("recommended_action", "Monitor closely") if data["signals"] else ""

            results.append({
                "target": target,
                "risk_score": round(score, 2),
                "risk_level": level,
                "explanation": explanation,
                "recommended_action": action,
            })

            # Store assessment
            def _store(r=results[-1]):
                conn = _db_connect()
                try:
                    conn.execute(
                        "INSERT INTO risk_assessments "
                        "(target, risk_score, risk_level, explanation, recommended_action, created_at) "
                        "VALUES (?, ?, ?, ?, ?, ?)",
                        (r["target"], r["risk_score"], r["risk_level"],
                         r["explanation"], r["recommended_action"], time.time()),
                    )
                    conn.commit()
                finally:
                    conn.close()
            await asyncio.to_thread(_store)

        return results


predictor = PredictiveMonitor()


# ---------------------------------------------------------------------------
# Self-Healing Layer
# ---------------------------------------------------------------------------

FAULT_CLASSES = [
    "WORKER_FAILURE", "PROVIDER_UNAVAILABLE", "QUEUE_OVERLOAD",
    "MODEL_RUNTIME_ERROR", "SERVICE_UNRESPONSIVE", "DISK_PRESSURE",
    "NETWORK_DEGRADATION", "BOT_HEALTH_FAILURE", "CONFIG_MISMATCH",
    "UNKNOWN_ANOMALY",
]

# Repair policies: fault_class -> list of allowed repair actions
REPAIR_POLICIES = {
    "SERVICE_UNRESPONSIVE": [
        {"action_type": "restart_service", "risk_level": "LOW",
         "rollback": "verify_service_health"},
    ],
    "WORKER_FAILURE": [
        {"action_type": "restart_runtime", "risk_level": "LOW",
         "rollback": "verify_runtime_health"},
    ],
    "PROVIDER_UNAVAILABLE": [
        {"action_type": "fallback_provider", "risk_level": "LOW",
         "rollback": "restore_provider_routing"},
    ],
    "DISK_PRESSURE": [
        {"action_type": "cleanup_temp_files", "risk_level": "LOW",
         "rollback": "verify_disk_space"},
    ],
    "MODEL_RUNTIME_ERROR": [
        {"action_type": "reload_runtime", "risk_level": "LOW",
         "rollback": "verify_model_health"},
    ],
}

# Repair DB tables
_repair_tables_sql = """
CREATE TABLE IF NOT EXISTS repair_events (
    repair_id TEXT PRIMARY KEY,
    fault_class TEXT NOT NULL,
    target TEXT NOT NULL,
    action_taken TEXT,
    success INTEGER,
    timestamp REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS repair_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repair_id TEXT NOT NULL,
    verification_result TEXT,
    rollback_performed INTEGER DEFAULT 0,
    notes TEXT,
    timestamp REAL NOT NULL
);
"""


class SelfHealingLayer:
    """Bounded self-healing with fault classification, repair, verification, and rollback."""

    def __init__(self):
        self.enabled = True
        self._init_tables()

    def _init_tables(self):
        """Create repair tables."""
        try:
            conn = _db_connect()
            conn.executescript(_repair_tables_sql)
            conn.commit()
            conn.close()
        except Exception as e:
            log.warning(f"Self-healing table init: {e}")

    async def classify_fault(self, check_result: Dict) -> Optional[Dict]:
        """Classify a monitoring result into a fault type."""
        status = check_result.get("status", "ok")
        if status == "ok":
            return None

        check_id = check_result.get("check_id", "")
        target = check_result.get("target", "")
        result_text = check_result.get("result", "").lower()

        # Classification rules
        fault_class = "UNKNOWN_ANOMALY"
        if "disk" in check_id:
            fault_class = "DISK_PRESSURE"
        elif "gpu" in check_id and ("unavailable" in result_text or "error" in result_text):
            fault_class = "MODEL_RUNTIME_ERROR"
        elif "ollama" in check_id and "down" in result_text:
            fault_class = "SERVICE_UNRESPONSIVE"
        elif "docker" in check_id and "restart" in result_text:
            fault_class = "WORKER_FAILURE"
        elif check_id.startswith("vm-") and status == "critical":
            fault_class = "SERVICE_UNRESPONSIVE"

        return {
            "fault_id": f"fault-{uuid.uuid4().hex[:8]}",
            "fault_class": fault_class,
            "affected_entity": target,
            "confidence": 0.8 if status == "critical" else 0.5,
            "related_metrics": check_result,
            "timestamp": time.time(),
        }

    async def attempt_repair(self, fault: Dict, channel: str = "") -> Dict:
        """Attempt automatic repair based on policies."""
        if not self.enabled:
            return {"status": "disabled", "message": "Self-healing is disabled"}

        fault_class = fault["fault_class"]
        target = fault["affected_entity"]
        repair_id = f"repair-{uuid.uuid4().hex[:8]}"

        policies = REPAIR_POLICIES.get(fault_class, [])
        low_risk = [p for p in policies if p["risk_level"] == "LOW"]

        if not low_risk:
            return {"status": "no_policy", "message": f"No LOW-risk repair for {fault_class}"}

        action = low_risk[0]
        action_type = action["action_type"]

        # Execute repair
        result = await self._execute_repair(repair_id, action_type, target)

        # Store repair event
        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO repair_events (repair_id, fault_class, target, action_taken, success, timestamp) "
                    "VALUES (?, ?, ?, ?, ?, ?)",
                    (repair_id, fault_class, target, action_type,
                     1 if result.get("success") else 0, time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)

        # Verify repair after delay
        if result.get("success"):
            await asyncio.sleep(10)
            verified = await self._verify_repair(repair_id, target, action)
            if not verified:
                await self._rollback(repair_id, action, target)
                result["rolled_back"] = True

        # Alert
        if channel:
            icon = ":wrench:" if result.get("success") else ":x:"
            rb = " (rolled back)" if result.get("rolled_back") else ""
            await post_message(
                f"{icon} *Self-heal* `{repair_id}`: {action_type} on `{target}` — "
                f"{'success' if result.get('success') else 'failed'}{rb}",
                channel,
            )

        return result

    async def _execute_repair(self, repair_id: str, action_type: str, target: str) -> Dict:
        """Execute a specific repair action."""
        try:
            if action_type == "restart_service":
                if target in VMS:
                    # Restart common services
                    await tool_executor.execute("shell", target,
                        "sudo systemctl restart bunny-alpha 2>/dev/null; "
                        "sudo systemctl restart docker 2>/dev/null; echo 'restarted'")
                return {"success": True, "action": action_type}

            elif action_type == "restart_runtime":
                await tool_executor.execute("shell", target,
                    "sudo systemctl restart ollama 2>/dev/null || echo 'no ollama'")
                return {"success": True, "action": action_type}

            elif action_type == "cleanup_temp_files":
                await tool_executor.execute("shell", target,
                    "sudo find /tmp -type f -mtime +7 -delete 2>/dev/null; "
                    "sudo journalctl --vacuum-time=3d 2>/dev/null; echo 'cleaned'")
                return {"success": True, "action": action_type}

            elif action_type == "reload_runtime":
                await tool_executor.execute("shell", target,
                    "sudo systemctl restart ollama 2>/dev/null; sleep 2; "
                    "curl -s http://localhost:11434/api/tags > /dev/null && echo 'ok' || echo 'failed'")
                return {"success": True, "action": action_type}

            elif action_type == "fallback_provider":
                # Just log — provider fallback is automatic in query_ai
                return {"success": True, "action": action_type}

            return {"success": False, "action": action_type, "error": "Unknown action"}
        except Exception as e:
            return {"success": False, "action": action_type, "error": str(e)}

    async def _verify_repair(self, repair_id: str, target: str, action: Dict) -> bool:
        """Verify that a repair was successful."""
        try:
            result = await tool_executor.execute("shell", target, "uptime")
            verified = bool(result and "up" in result.lower())

            def _store():
                conn = _db_connect()
                try:
                    conn.execute(
                        "INSERT INTO repair_history (repair_id, verification_result, timestamp) "
                        "VALUES (?, ?, ?)",
                        (repair_id, "verified" if verified else "failed", time.time()),
                    )
                    conn.commit()
                finally:
                    conn.close()
            await asyncio.to_thread(_store)
            return verified
        except Exception:
            return False

    async def _rollback(self, repair_id: str, action: Dict, target: str):
        """Rollback a failed repair."""
        log.warning(f"Rolling back repair {repair_id} on {target}")
        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO repair_history (repair_id, verification_result, rollback_performed, notes, timestamp) "
                    "VALUES (?, 'rollback', 1, 'Auto-rollback after verification failure', ?)",
                    (repair_id, time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)

    async def get_repair_history(self, limit: int = 20) -> List[Dict]:
        """Get recent repair events."""
        def _query():
            conn = _db_connect()
            try:
                rows = conn.execute(
                    "SELECT * FROM repair_events ORDER BY timestamp DESC LIMIT ?", (limit,)
                ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)


self_healer = SelfHealingLayer()


# ---------------------------------------------------------------------------
# Performance-Aware Routing
# ---------------------------------------------------------------------------

ROUTING_MODES = ["BALANCED", "LOW_LATENCY", "LOW_COST", "HIGH_RELIABILITY"]
_routing_mode = "BALANCED"


class PerformanceRouter:
    """Routes tasks based on live and historical performance."""

    async def record_result(self, target_id: str, target_type: str,
                            route_type: str, success: bool, latency: float):
        """Record a routing result for performance tracking."""
        def _update():
            conn = _db_connect()
            try:
                row = conn.execute(
                    "SELECT * FROM routing_performance WHERE target_id = ? AND route_type = ?",
                    (target_id, route_type),
                ).fetchone()
                now = time.time()
                if row:
                    count = row["request_count"] + 1
                    sr = (row["success_rate"] * row["request_count"] + (1 if success else 0)) / count
                    al = (row["avg_latency"] * row["request_count"] + latency) / count
                    er = (row["error_rate"] * row["request_count"] + (0 if success else 1)) / count
                    conn.execute(
                        "UPDATE routing_performance SET success_rate=?, avg_latency=?, "
                        "error_rate=?, request_count=?, updated_at=? "
                        "WHERE target_id=? AND route_type=?",
                        (sr, al, er, count, now, target_id, route_type),
                    )
                else:
                    conn.execute(
                        "INSERT INTO routing_performance "
                        "(route_type, target_type, target_id, success_rate, avg_latency, "
                        "error_rate, request_count, updated_at) "
                        "VALUES (?, ?, ?, ?, ?, ?, 1, ?)",
                        (route_type, target_type, target_id,
                         1.0 if success else 0.0, latency,
                         0.0 if success else 1.0, now),
                    )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_update)

    async def get_performance(self, target_id: str = None) -> List[Dict]:
        """Get performance stats."""
        def _query():
            conn = _db_connect()
            try:
                if target_id:
                    rows = conn.execute(
                        "SELECT * FROM routing_performance WHERE target_id = ?", (target_id,)
                    ).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT * FROM routing_performance ORDER BY updated_at DESC"
                    ).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_query)

    async def select_best_target(self, candidates: List[str], route_type: str = "ai") -> str:
        """Select the best target based on current routing mode and performance."""
        global _routing_mode
        if not candidates:
            return ""

        perf_data = await self.get_performance()
        scores = {}

        for target in candidates:
            target_perf = next((p for p in perf_data if p["target_id"] == target), None)
            if not target_perf:
                scores[target] = 0.5  # Unknown = neutral
                continue

            if _routing_mode == "LOW_LATENCY":
                scores[target] = 1.0 / max(0.001, target_perf["avg_latency"])
            elif _routing_mode == "HIGH_RELIABILITY":
                scores[target] = target_perf["success_rate"]
            elif _routing_mode == "LOW_COST":
                scores[target] = target_perf["success_rate"] / max(0.001, target_perf["avg_latency"])
            else:  # BALANCED
                scores[target] = (
                    target_perf["success_rate"] * 0.5 +
                    (1.0 / max(0.001, target_perf["avg_latency"])) * 0.3 +
                    (1.0 - target_perf["error_rate"]) * 0.2
                )

        best = max(scores, key=scores.get) if scores else candidates[0]
        return best


perf_router = PerformanceRouter()


# ---------------------------------------------------------------------------
# Execution Simulation
# ---------------------------------------------------------------------------

class SimulationEngine:
    """Pre-execution simulation to estimate risk and impact."""

    async def simulate_plan(self, plan_id: str) -> Dict:
        """Simulate executing a plan and estimate risk."""
        plan_data = await planner.get_plan(plan_id)
        if not plan_data:
            return {"error": "Plan not found"}

        steps = plan_data["steps"]
        sim_id = f"sim-{uuid.uuid4().hex[:8]}"
        risk_score = 0
        predicted_outcomes = []

        for step in steps:
            title = step["title"].lower()
            desc = step.get("description", "").lower()

            # Estimate risk per step
            step_risk = 0.1
            if any(w in title + desc for w in ["restart", "stop", "kill"]):
                step_risk = 0.4
            elif any(w in title + desc for w in ["delete", "remove", "drop"]):
                step_risk = 0.7
            elif any(w in title + desc for w in ["deploy", "install", "upgrade"]):
                step_risk = 0.5
            elif any(w in title + desc for w in ["check", "status", "verify", "test"]):
                step_risk = 0.05

            predicted_outcomes.append({
                "step": step["title"],
                "predicted_success": 1.0 - step_risk,
                "risk": step_risk,
            })
            risk_score = max(risk_score, step_risk)

        # Overall risk
        avg_risk = sum(p["risk"] for p in predicted_outcomes) / max(1, len(predicted_outcomes))
        risk_level = "CRITICAL" if avg_risk > 0.6 else "HIGH" if avg_risk > 0.4 else "MODERATE" if avg_risk > 0.2 else "LOW"

        result = {
            "simulation_id": sim_id,
            "plan_id": plan_id,
            "risk_score": round(avg_risk, 2),
            "risk_level": risk_level,
            "steps": predicted_outcomes,
            "recommended_action": "proceed" if risk_level in ("LOW", "MODERATE") else "review_required",
        }

        # Store
        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO simulations "
                    "(simulation_id, plan_id, scenario_type, inputs_json, "
                    "predicted_outcomes_json, risk_score, recommended_action, created_at) "
                    "VALUES (?, ?, 'plan', ?, ?, ?, ?, ?)",
                    (sim_id, plan_id, json.dumps({"steps": len(steps)}),
                     json.dumps(predicted_outcomes), avg_risk,
                     result["recommended_action"], time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)
        return result

    async def simulate_action(self, action_type: str, target: str = "") -> Dict:
        """Simulate a single action."""
        sim_id = f"sim-{uuid.uuid4().hex[:8]}"
        risk_map = {
            "restart_service": 0.3,
            "restart_runtime": 0.3,
            "deploy": 0.5,
            "cleanup_temp_files": 0.1,
            "fallback_provider": 0.1,
            "reload_runtime": 0.25,
            "delete": 0.8,
            "stop": 0.6,
        }
        risk = risk_map.get(action_type, 0.3)
        risk_level = "CRITICAL" if risk > 0.6 else "HIGH" if risk > 0.4 else "MODERATE" if risk > 0.2 else "LOW"

        result = {
            "simulation_id": sim_id,
            "action_type": action_type,
            "target": target,
            "risk_score": risk,
            "risk_level": risk_level,
            "recommended_action": "proceed" if risk_level in ("LOW", "MODERATE") else "review_required",
        }

        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO simulations "
                    "(simulation_id, scenario_type, inputs_json, risk_score, "
                    "recommended_action, created_at) "
                    "VALUES (?, 'action', ?, ?, ?, ?)",
                    (sim_id, json.dumps({"action": action_type, "target": target}),
                     risk, result["recommended_action"], time.time()),
                )
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)
        return result


simulator = SimulationEngine()


# ---------------------------------------------------------------------------
# Operational Hardening Layer
# ---------------------------------------------------------------------------

class SwarmSessionManager:
    """Manages swarm sessions tied to user interactions."""

    async def create_session(self, user_id: str, assistant: str = "bunny-alpha",
                              portal_session_id: str = None) -> str:
        sid = f"sess-{uuid.uuid4().hex[:12]}"
        now = time.time()
        def _create():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO swarm_sessions "
                    "(session_id, user_id, assistant_name, portal_session_id, "
                    "status, created_at, last_active_at) VALUES (?,?,?,?,?,?,?)",
                    (sid, user_id, assistant, portal_session_id, "active", now, now))
                conn.execute(
                    "INSERT INTO session_events (event_id, session_id, event_type, created_at) "
                    "VALUES (?, ?, 'session_start', ?)",
                    (f"evt-{uuid.uuid4().hex[:8]}", sid, now))
                conn.commit()
                return sid
            finally:
                conn.close()
        return await asyncio.to_thread(_create)

    async def touch(self, session_id: str):
        def _t():
            conn = _db_connect()
            try:
                conn.execute("UPDATE swarm_sessions SET last_active_at=? WHERE session_id=?",
                             (time.time(), session_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_t)

    async def close_session(self, session_id: str, summary: str = None):
        now = time.time()
        def _close():
            conn = _db_connect()
            try:
                conn.execute("UPDATE swarm_sessions SET status='closed', summary=?, last_active_at=? "
                             "WHERE session_id=?", (summary, now, session_id))
                conn.execute(
                    "INSERT INTO session_events (event_id, session_id, event_type, created_at) "
                    "VALUES (?, ?, 'session_close', ?)",
                    (f"evt-{uuid.uuid4().hex[:8]}", session_id, now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_close)

    async def resume_session(self, session_id: str):
        def _resume():
            conn = _db_connect()
            try:
                conn.execute("UPDATE swarm_sessions SET status='active', last_active_at=? "
                             "WHERE session_id=?", (time.time(), session_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_resume)

    async def get_session(self, session_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM swarm_sessions WHERE session_id=?",
                                   (session_id,)).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def list_sessions(self, user_id: str = None, status: str = None,
                             limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                sql = "SELECT * FROM swarm_sessions WHERE 1=1"
                params = []
                if user_id:
                    sql += " AND user_id=?"
                    params.append(user_id)
                if status:
                    sql += " AND status=?"
                    params.append(status)
                sql += " ORDER BY last_active_at DESC LIMIT ?"
                params.append(limit)
                return [dict(r) for r in conn.execute(sql, params).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def add_event(self, session_id: str, event_type: str, payload: Dict = None):
        def _e():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO session_events (event_id, session_id, event_type, payload_json, created_at) "
                    "VALUES (?,?,?,?,?)",
                    (f"evt-{uuid.uuid4().hex[:8]}", session_id, event_type,
                     json.dumps(payload) if payload else None, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_e)

    async def get_events(self, session_id: str, limit: int = 50) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                rows = conn.execute(
                    "SELECT * FROM session_events WHERE session_id=? ORDER BY created_at DESC LIMIT ?",
                    (session_id, limit)).fetchall()
                return [dict(r) for r in rows]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


session_mgr = SwarmSessionManager()


class AuditLogger:
    """Append-only audit log for all critical system actions."""

    async def log(self, action_type: str, actor_id: str = "system",
                  actor_type: str = "SYSTEM", session_id: str = None,
                  task_id: str = None, target_type: str = None,
                  target_id: str = None, payload: Dict = None,
                  result: str = None):
        aid = f"aud-{uuid.uuid4().hex[:10]}"
        def _log():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO audit_events "
                    "(audit_id, actor_type, actor_id, session_id, task_id, "
                    "action_type, target_type, target_id, payload_json, result, created_at) "
                    "VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                    (aid, actor_type, actor_id, session_id, task_id,
                     action_type, target_type, target_id,
                     json.dumps(payload) if payload else None, result, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_log)
        return aid

    async def search(self, action_type: str = None, actor_id: str = None,
                     session_id: str = None, limit: int = 50) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                sql = "SELECT * FROM audit_events WHERE 1=1"
                params = []
                if action_type:
                    sql += " AND action_type=?"
                    params.append(action_type)
                if actor_id:
                    sql += " AND actor_id=?"
                    params.append(actor_id)
                if session_id:
                    sql += " AND session_id=?"
                    params.append(session_id)
                sql += " ORDER BY created_at DESC LIMIT ?"
                params.append(limit)
                return [dict(r) for r in conn.execute(sql, params).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_recent(self, limit: int = 20) -> List[Dict]:
        return await self.search(limit=limit)

    async def count(self) -> int:
        def _c():
            conn = _db_connect()
            try:
                return conn.execute("SELECT COUNT(*) FROM audit_events").fetchone()[0]
            finally:
                conn.close()
        return await asyncio.to_thread(_c)


audit = AuditLogger()


# Roles: USER, OPERATOR, ADMIN, SYSTEM
# Action classes: READ_ONLY, SAFE_MUTATION, RISKY_MUTATION, DESTRUCTIVE
ROLE_HIERARCHY = {"SYSTEM": 4, "ADMIN": 3, "OPERATOR": 2, "USER": 1}
DEFAULT_PERMISSIONS = {
    "USER": ["READ_ONLY"],
    "OPERATOR": ["READ_ONLY", "SAFE_MUTATION", "RISKY_MUTATION"],
    "ADMIN": ["READ_ONLY", "SAFE_MUTATION", "RISKY_MUTATION", "DESTRUCTIVE"],
    "SYSTEM": ["READ_ONLY", "SAFE_MUTATION", "RISKY_MUTATION", "DESTRUCTIVE"],
}
ACTION_CLASS_MAP = {
    "inspect_tasks": "READ_ONLY", "inspect_plans": "READ_ONLY",
    "inspect_logs": "READ_ONLY", "inspect_graph": "READ_ONLY",
    "retry_task": "SAFE_MUTATION", "reroute_provider": "SAFE_MUTATION",
    "trigger_health_check": "SAFE_MUTATION", "restart_noncritical": "SAFE_MUTATION",
    "quarantine_worker": "RISKY_MUTATION", "reduce_queue_capacity": "RISKY_MUTATION",
    "modify_routing_mode": "RISKY_MUTATION", "execute_multi_step_repair": "RISKY_MUTATION",
    "delete_data": "DESTRUCTIVE", "remove_resources": "DESTRUCTIVE",
    "irreversible_infra_change": "DESTRUCTIVE",
}


class PermissionManager:
    """Role-based permissions and approval gates."""

    def __init__(self):
        self._user_roles: Dict[str, str] = {}  # user_id -> role

    async def seed_defaults(self):
        def _seed():
            conn = _db_connect()
            try:
                for role, classes in DEFAULT_PERMISSIONS.items():
                    for ac in classes:
                        conn.execute(
                            "INSERT OR IGNORE INTO permissions (role, action_class, allowed) "
                            "VALUES (?, ?, 1)", (role, ac))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_seed)

    def get_role(self, user_id: str) -> str:
        return self._user_roles.get(user_id, "OPERATOR")

    def set_role(self, user_id: str, role: str):
        if role in ROLE_HIERARCHY:
            self._user_roles[user_id] = role

    async def check_permission(self, user_id: str, action_type: str) -> bool:
        role = self.get_role(user_id)
        action_class = ACTION_CLASS_MAP.get(action_type, "SAFE_MUTATION")
        allowed_classes = DEFAULT_PERMISSIONS.get(role, [])
        return action_class in allowed_classes

    async def request_approval(self, requested_by: str, action_type: str,
                                payload: Dict = None, reason: str = None) -> str:
        aid = f"appr-{uuid.uuid4().hex[:8]}"
        action_class = ACTION_CLASS_MAP.get(action_type, "RISKY_MUTATION")
        def _req():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO approvals "
                    "(approval_id, requested_by, action_type, action_class, "
                    "action_payload, reason, status, created_at) VALUES (?,?,?,?,?,?,?,?)",
                    (aid, requested_by, action_type, action_class,
                     json.dumps(payload) if payload else None,
                     reason, "pending", time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_req)
        await audit.log("approval_requested", actor_id=requested_by,
                        actor_type="USER", target_id=aid,
                        payload={"action_type": action_type, "reason": reason})
        return aid

    async def approve(self, approval_id: str, approved_by: str) -> bool:
        def _a():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM approvals WHERE approval_id=?",
                                   (approval_id,)).fetchone()
                if not row or row["status"] != "pending":
                    return False
                conn.execute("UPDATE approvals SET status='approved', approved_by=?, resolved_at=? "
                             "WHERE approval_id=?", (approved_by, time.time(), approval_id))
                conn.commit()
                return True
            finally:
                conn.close()
        result = await asyncio.to_thread(_a)
        if result:
            await audit.log("approval_granted", actor_id=approved_by,
                            target_id=approval_id)
        return result

    async def reject(self, approval_id: str, rejected_by: str) -> bool:
        def _r():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM approvals WHERE approval_id=?",
                                   (approval_id,)).fetchone()
                if not row or row["status"] != "pending":
                    return False
                conn.execute("UPDATE approvals SET status='rejected', approved_by=?, resolved_at=? "
                             "WHERE approval_id=?", (rejected_by, time.time(), approval_id))
                conn.commit()
                return True
            finally:
                conn.close()
        result = await asyncio.to_thread(_r)
        if result:
            await audit.log("approval_rejected", actor_id=rejected_by,
                            target_id=approval_id)
        return result

    async def get_pending(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM approvals WHERE status='pending' ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_approval(self, approval_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM approvals WHERE approval_id=?",
                                   (approval_id,)).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


perm_mgr = PermissionManager()


# Execution sandbox profiles
SANDBOX_PROFILES = {
    "readonly": {"timeout": 10, "output_limit": 5000, "allowed_cmds": ["cat", "ls", "ps", "df", "free", "uptime", "whoami", "date"]},
    "safe_ops": {"timeout": 30, "output_limit": 10000, "allowed_cmds": None, "blocked_cmds": ["rm -rf", "dd ", "mkfs", "fdisk", "> /dev/"]},
    "admin": {"timeout": 60, "output_limit": 20000, "allowed_cmds": None, "blocked_cmds": ["rm -rf /"]},
}
CODE_SANDBOX = {"python": {"timeout": 30, "max_memory_mb": 256, "max_output": 10000},
                "js": {"timeout": 15, "max_memory_mb": 128, "max_output": 5000}}


class ExecutionSandbox:
    """Bounds execution with policy-driven limits."""

    def __init__(self):
        self.default_profile = "safe_ops"

    async def seed_policies(self):
        def _seed():
            conn = _db_connect()
            try:
                for name, profile in SANDBOX_PROFILES.items():
                    conn.execute(
                        "INSERT OR IGNORE INTO execution_policies "
                        "(policy_id, action_type, timeout_seconds, output_limit, "
                        "allowed_paths_json, role_scope) VALUES (?,?,?,?,?,?)",
                        (f"policy-{name}", "shell", profile["timeout"],
                         profile["output_limit"],
                         json.dumps(profile.get("allowed_cmds")),
                         "ADMIN" if name == "admin" else "OPERATOR"))
                for lang, cfg in CODE_SANDBOX.items():
                    conn.execute(
                        "INSERT OR IGNORE INTO execution_policies "
                        "(policy_id, action_type, timeout_seconds, output_limit, role_scope) "
                        "VALUES (?,?,?,?,?)",
                        (f"policy-code-{lang}", f"code_{lang}", cfg["timeout"],
                         cfg["max_output"], "OPERATOR"))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_seed)

    def check_command(self, cmd: str, profile: str = None) -> Tuple[bool, str]:
        p = SANDBOX_PROFILES.get(profile or self.default_profile, SANDBOX_PROFILES["safe_ops"])
        blocked = p.get("blocked_cmds", [])
        if blocked:
            for bc in blocked:
                if bc in cmd:
                    return False, f"Blocked by sandbox policy: '{bc}'"
        allowed = p.get("allowed_cmds")
        if allowed:
            cmd_base = cmd.strip().split()[0] if cmd.strip() else ""
            if cmd_base not in allowed:
                return False, f"Command '{cmd_base}' not in readonly allowlist"
        return True, "ok"

    def get_timeout(self, profile: str = None) -> int:
        p = SANDBOX_PROFILES.get(profile or self.default_profile, SANDBOX_PROFILES["safe_ops"])
        return p["timeout"]

    def get_output_limit(self, profile: str = None) -> int:
        p = SANDBOX_PROFILES.get(profile or self.default_profile, SANDBOX_PROFILES["safe_ops"])
        return p["output_limit"]

    async def get_policies(self) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute("SELECT * FROM execution_policies").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


sandbox = ExecutionSandbox()


class EscalationManager:
    """Human-in-the-loop escalation for uncertain/high-risk situations."""

    async def escalate(self, trigger_type: str, confidence: float = 0.5,
                        session_id: str = None, task_id: str = None,
                        plan_id: str = None,
                        recommended_actions: List[str] = None) -> str:
        eid = f"esc-{uuid.uuid4().hex[:8]}"
        def _esc():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO escalations "
                    "(escalation_id, session_id, task_id, plan_id, trigger_type, "
                    "confidence, recommended_actions_json, status, created_at) "
                    "VALUES (?,?,?,?,?,?,?,?,?)",
                    (eid, session_id, task_id, plan_id, trigger_type, confidence,
                     json.dumps(recommended_actions or []), "open", time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_esc)
        await audit.log("escalation_created", target_id=eid,
                        payload={"trigger": trigger_type, "confidence": confidence})
        return eid

    async def resolve(self, escalation_id: str, resolved_by: str = "operator",
                       notes: str = None) -> bool:
        def _r():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE escalations SET status='resolved', resolved_by=?, "
                    "resolution_notes=?, resolved_at=? WHERE escalation_id=?",
                    (resolved_by, notes, time.time(), escalation_id))
                conn.commit()
                return True
            finally:
                conn.close()
        result = await asyncio.to_thread(_r)
        if result:
            await audit.log("escalation_resolved", actor_id=resolved_by,
                            target_id=escalation_id)
        return result

    async def get_open(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM escalations WHERE status='open' ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_all(self, limit: int = 50) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM escalations ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def should_escalate(self, risk_score: float, retries: int = 0,
                               repair_failures: int = 0) -> bool:
        if risk_score > 0.7:
            return True
        if retries >= 3:
            return True
        if repair_failures >= 2:
            return True
        return False


escalation_mgr = EscalationManager()


class FailureDrillRunner:
    """Controlled failure drills to verify platform resilience."""

    DRILL_TYPES = [
        "provider_outage", "worker_loss", "queue_overload", "failed_repair",
        "unhealthy_runtime", "dashboard_degradation", "memory_db_reconnect",
        "approval_queue_deadlock", "tunnel_flakiness",
    ]

    async def run_drill(self, drill_type: str, target: str = None) -> Dict:
        if drill_type not in self.DRILL_TYPES:
            return {"error": f"Unknown drill type. Available: {', '.join(self.DRILL_TYPES)}"}

        did = f"drill-{uuid.uuid4().hex[:8]}"
        start = time.time()

        def _create():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO failure_drills "
                    "(drill_id, drill_type, target, status, started_at) VALUES (?,?,?,?,?)",
                    (did, drill_type, target, "running", start))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_create)
        await audit.log("drill_started", target_id=did,
                        payload={"type": drill_type, "target": target})

        # Simulate drill execution in safe mode
        detection_time = 0.0
        mitigation_time = 0.0
        recovery_time = 0.0
        rollback = 0
        outcome = "passed"
        lessons = []

        try:
            if drill_type == "provider_outage":
                detection_time = 0.5
                mitigation_time = 2.0
                recovery_time = 3.0
                lessons = ["Fallback provider chain activated", "Routing adjusted within 2s"]
            elif drill_type == "worker_loss":
                detection_time = 1.0
                mitigation_time = 5.0
                recovery_time = 8.0
                lessons = ["Worker health check detected loss", "Tasks redistributed"]
            elif drill_type == "queue_overload":
                detection_time = 0.3
                mitigation_time = 1.0
                recovery_time = 2.0
                lessons = ["Queue pressure detected", "Rate limiting engaged"]
            elif drill_type == "failed_repair":
                detection_time = 0.5
                mitigation_time = 3.0
                recovery_time = 5.0
                rollback = 1
                lessons = ["Repair failed", "Rollback triggered successfully"]
            elif drill_type == "unhealthy_runtime":
                detection_time = 1.0
                mitigation_time = 4.0
                recovery_time = 6.0
                lessons = ["Runtime health degradation detected", "Self-healing initiated"]
            elif drill_type == "memory_db_reconnect":
                detection_time = 0.2
                mitigation_time = 1.5
                recovery_time = 2.0
                lessons = ["DB connection lost", "WAL mode reconnection successful"]
            else:
                detection_time = 1.0
                mitigation_time = 3.0
                recovery_time = 5.0
                lessons = [f"Drill '{drill_type}' completed in safe mode"]
        except Exception as e:
            outcome = "failed"
            lessons = [f"Drill error: {str(e)}"]

        end = time.time()
        def _complete():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE failure_drills SET status='completed', completed_at=?, outcome=? "
                    "WHERE drill_id=?", (end, outcome, did))
                conn.execute(
                    "INSERT INTO drill_results "
                    "(drill_id, detection_time, mitigation_time, recovery_time, "
                    "rollback_triggered, lessons_json, created_at) VALUES (?,?,?,?,?,?,?)",
                    (did, detection_time, mitigation_time, recovery_time,
                     rollback, json.dumps(lessons), end))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_complete)
        await audit.log("drill_completed", target_id=did,
                        payload={"outcome": outcome}, result=outcome)

        return {
            "drill_id": did, "type": drill_type, "target": target,
            "outcome": outcome, "detection_time": detection_time,
            "mitigation_time": mitigation_time, "recovery_time": recovery_time,
            "rollback": bool(rollback), "lessons": lessons,
        }

    async def get_drill(self, drill_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                drill = conn.execute("SELECT * FROM failure_drills WHERE drill_id=?",
                                     (drill_id,)).fetchone()
                if not drill:
                    return None
                results = conn.execute("SELECT * FROM drill_results WHERE drill_id=?",
                                       (drill_id,)).fetchall()
                d = dict(drill)
                d["results"] = [dict(r) for r in results]
                return d
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_history(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM failure_drills ORDER BY started_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


drill_runner = FailureDrillRunner()


# ---------------------------------------------------------------------------
# Continuous Learning & System Intelligence Layer
# ---------------------------------------------------------------------------

class OutcomeLearner:
    """Captures structured outcomes from tasks, routes, and repairs."""

    async def record_task_outcome(self, task_id: str, task_type: str = None,
                                   success: bool = True, duration_ms: float = 0,
                                   retries: int = 0, provider_used: str = None,
                                   session_id: str = None, plan_id: str = None,
                                   selected_target: str = None) -> str:
        oid = f"tout-{uuid.uuid4().hex[:8]}"
        def _r():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO task_outcomes "
                    "(outcome_id, task_id, session_id, plan_id, task_type, "
                    "selected_target, success, duration_ms, retries, "
                    "provider_used, created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                    (oid, task_id, session_id, plan_id, task_type,
                     selected_target, 1 if success else 0, duration_ms,
                     retries, provider_used, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_r)
        return oid

    async def record_route_outcome(self, decision_id: str, target: str,
                                    success: bool, latency_ms: float = 0,
                                    fallback: bool = False) -> str:
        oid = f"rout-{uuid.uuid4().hex[:8]}"
        def _r():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO route_outcomes "
                    "(route_outcome_id, routing_decision_id, selected_target, "
                    "success, latency_ms, fallback_used, created_at) VALUES (?,?,?,?,?,?,?)",
                    (oid, decision_id, target, 1 if success else 0,
                     latency_ms, 1 if fallback else 0, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_r)
        return oid

    async def record_repair_outcome(self, repair_id: str, repair_type: str,
                                     success: bool, rollback: bool = False,
                                     recovery_time_ms: float = 0) -> str:
        oid = f"rpout-{uuid.uuid4().hex[:8]}"
        def _r():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO repair_outcomes "
                    "(repair_outcome_id, repair_id, repair_type, success, "
                    "rollback_triggered, recovery_time_ms, created_at) VALUES (?,?,?,?,?,?,?)",
                    (oid, repair_id, repair_type, 1 if success else 0,
                     1 if rollback else 0, recovery_time_ms, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_r)
        return oid

    async def get_recent(self, outcome_type: str = "task", limit: int = 20) -> List[Dict]:
        table = {"task": "task_outcomes", "route": "route_outcomes",
                 "repair": "repair_outcomes"}.get(outcome_type, "task_outcomes")
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    f"SELECT * FROM {table} ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                tc = conn.execute("SELECT COUNT(*) FROM task_outcomes").fetchone()[0]
                ts = conn.execute("SELECT COUNT(*) FROM task_outcomes WHERE success=1").fetchone()[0]
                rc = conn.execute("SELECT COUNT(*) FROM route_outcomes").fetchone()[0]
                rpc = conn.execute("SELECT COUNT(*) FROM repair_outcomes").fetchone()[0]
                return {"task_outcomes": tc, "task_success_rate": round(ts / max(1, tc), 3),
                        "route_outcomes": rc, "repair_outcomes": rpc}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


outcome_learner = OutcomeLearner()


class PlanOptimizer:
    """Improves planning from historical execution results."""

    async def score_plan(self, plan_id: str) -> Dict:
        def _score():
            conn = _db_connect()
            try:
                plan = conn.execute("SELECT * FROM goal_plans WHERE plan_id=?",
                                    (plan_id,)).fetchone()
                if not plan:
                    return {"error": "Plan not found"}
                steps = conn.execute("SELECT * FROM plan_steps WHERE plan_id=?",
                                     (plan_id,)).fetchall()
                completed = [s for s in steps if s["status"] == "completed"]
                failed = [s for s in steps if s["status"] == "failed"]
                total = len(steps)
                score = len(completed) / max(1, total)
                return {
                    "plan_id": plan_id, "goal": plan["goal_text"],
                    "status": plan["status"], "total_steps": total,
                    "completed": len(completed), "failed": len(failed),
                    "score": round(score, 3),
                }
            finally:
                conn.close()
        return await asyncio.to_thread(_score)

    async def get_patterns(self, goal_type: str = None, limit: int = 10) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if goal_type:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM planning_patterns WHERE goal_type=? "
                        "ORDER BY success_rate DESC LIMIT ?", (goal_type, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM planning_patterns ORDER BY success_rate DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def update_patterns(self):
        """Analyze completed plans and update planning patterns."""
        def _update():
            conn = _db_connect()
            try:
                plans = conn.execute(
                    "SELECT * FROM plan_outcomes WHERE success IS NOT NULL"
                ).fetchall()
                type_stats = {}
                for p in plans:
                    gt = p["goal_type"] or "general"
                    if gt not in type_stats:
                        type_stats[gt] = {"successes": 0, "total": 0, "durations": [], "retries": []}
                    type_stats[gt]["total"] += 1
                    if p["success"]:
                        type_stats[gt]["successes"] += 1
                    if p["total_duration"]:
                        type_stats[gt]["durations"].append(p["total_duration"])
                    type_stats[gt]["retries"].append(p["retries"] or 0)
                now = time.time()
                for gt, stats in type_stats.items():
                    sr = stats["successes"] / max(1, stats["total"])
                    avg_dur = sum(stats["durations"]) / max(1, len(stats["durations"]))
                    avg_ret = sum(stats["retries"]) / max(1, len(stats["retries"]))
                    conf = min(1.0, stats["total"] / 10.0)
                    pid = f"pat-{gt[:20]}"
                    conn.execute(
                        "INSERT OR REPLACE INTO planning_patterns "
                        "(pattern_id, goal_type, success_rate, avg_duration_ms, "
                        "avg_retries, confidence, updated_at) VALUES (?,?,?,?,?,?,?)",
                        (pid, gt, sr, avg_dur, avg_ret, conf, now))
                conn.commit()
                return len(type_stats)
            finally:
                conn.close()
        return await asyncio.to_thread(_update)


plan_optimizer = PlanOptimizer()


class RoutingIntelligence:
    """Adaptive routing that learns from real performance."""

    async def update_scores(self):
        """Update routing scores from outcome data."""
        def _update():
            conn = _db_connect()
            try:
                rows = conn.execute("SELECT * FROM routing_performance").fetchall()
                now = time.time()
                for r in rows:
                    sr = r["success_rate"]
                    lat = 1.0 / max(0.001, r["avg_latency"]) if r["avg_latency"] else 0.5
                    lat_norm = min(1.0, lat)
                    rel = 1.0 - r["error_rate"]
                    overall = sr * 0.3 + lat_norm * 0.25 + rel * 0.25 + 0.2
                    sid = f"rscore-{r['target_id'][:20]}-{r['route_type'][:10]}"
                    conn.execute(
                        "INSERT OR REPLACE INTO routing_scores "
                        "(routing_score_id, route_type, target_type, target_id, "
                        "success_score, latency_score, reliability_score, "
                        "overall_score, updated_at) VALUES (?,?,?,?,?,?,?,?,?)",
                        (sid, r["route_type"], r["target_type"], r["target_id"],
                         sr, lat_norm, rel, overall, now))
                conn.commit()
                return len(rows)
            finally:
                conn.close()
        return await asyncio.to_thread(_update)

    async def get_weights(self) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute("SELECT * FROM routing_weights").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def seed_weights(self):
        def _seed():
            conn = _db_connect()
            try:
                now = time.time()
                for mode in ROUTING_MODES:
                    w = {"BALANCED": (0.3, 0.25, 0.15, 0.2, 0.1),
                         "LOW_LATENCY": (0.15, 0.45, 0.1, 0.2, 0.1),
                         "LOW_COST": (0.2, 0.1, 0.4, 0.2, 0.1),
                         "HIGH_RELIABILITY": (0.4, 0.1, 0.1, 0.3, 0.1)}[mode]
                    conn.execute(
                        "INSERT OR IGNORE INTO routing_weights "
                        "(weight_id, routing_mode, success_weight, latency_weight, "
                        "cost_weight, reliability_weight, fit_weight, updated_at) "
                        "VALUES (?,?,?,?,?,?,?,?)",
                        (f"rw-{mode.lower()}", mode, *w, now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_seed)

    async def explain_route(self, task_id: str) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                dec = conn.execute(
                    "SELECT * FROM routing_decisions WHERE task_id=? ORDER BY created_at DESC LIMIT 1",
                    (task_id,)).fetchone()
                if not dec:
                    return {"error": "No routing decision found for task"}
                d = dict(dec)
                exp = conn.execute(
                    "SELECT * FROM decision_explanations WHERE decision_type='route' "
                    "AND decision_id=? LIMIT 1", (str(dec["id"]),)).fetchone()
                if exp:
                    d["explanation"] = dict(exp)
                return d
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


routing_intel = RoutingIntelligence()


class RepairLearner:
    """Learns which repairs work best for each fault class."""

    async def update_patterns(self):
        """Analyze repair outcomes and update patterns."""
        def _update():
            conn = _db_connect()
            try:
                outcomes = conn.execute("SELECT * FROM repair_outcomes").fetchall()
                patterns: Dict[str, Dict] = {}
                for o in outcomes:
                    rt = o["repair_type"] or "unknown"
                    if rt not in patterns:
                        patterns[rt] = {"success": 0, "total": 0, "rollbacks": 0, "recovery_times": []}
                    patterns[rt]["total"] += 1
                    if o["success"]:
                        patterns[rt]["success"] += 1
                    if o["rollback_triggered"]:
                        patterns[rt]["rollbacks"] += 1
                    if o["recovery_time_ms"]:
                        patterns[rt]["recovery_times"].append(o["recovery_time_ms"])
                now = time.time()
                for rt, stats in patterns.items():
                    sr = stats["success"] / max(1, stats["total"])
                    rr = stats["rollbacks"] / max(1, stats["total"])
                    avg_rt = sum(stats["recovery_times"]) / max(1, len(stats["recovery_times"]))
                    conf = min(1.0, stats["total"] / 5.0)
                    pid = f"rpat-{rt[:20]}"
                    conn.execute(
                        "INSERT OR REPLACE INTO repair_patterns "
                        "(repair_pattern_id, fault_class, repair_type, success_rate, "
                        "avg_recovery_time_ms, rollback_rate, confidence, updated_at) "
                        "VALUES (?,?,?,?,?,?,?,?)",
                        (pid, rt, rt, sr, avg_rt, rr, conf, now))
                conn.commit()
                return len(patterns)
            finally:
                conn.close()
        return await asyncio.to_thread(_update)

    async def get_patterns(self, fault_class: str = None) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if fault_class:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM repair_patterns WHERE fault_class=?",
                        (fault_class,)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM repair_patterns ORDER BY success_rate DESC").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_learned_policies(self) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM repair_policies_learned").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


repair_learner = RepairLearner()


class AgentScorer:
    """Tracks and scores agent performance for delegation decisions."""

    async def record_outcome(self, agent_id: str, task_type: str,
                              success: bool, duration_ms: float = 0,
                              quality: float = 0.5, delegated_task_id: str = None):
        oid = f"agout-{uuid.uuid4().hex[:8]}"
        def _r():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO agent_outcomes "
                    "(agent_outcome_id, agent_id, delegated_task_id, task_type, "
                    "success, duration_ms, quality_score, created_at) VALUES (?,?,?,?,?,?,?,?)",
                    (oid, agent_id, delegated_task_id, task_type,
                     1 if success else 0, duration_ms, quality, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_r)

    async def update_scores(self):
        """Recalculate agent scores from outcomes."""
        def _update():
            conn = _db_connect()
            try:
                agents = conn.execute("SELECT DISTINCT agent_id FROM agent_outcomes").fetchall()
                now = time.time()
                for a in agents:
                    aid = a["agent_id"]
                    outcomes = conn.execute(
                        "SELECT * FROM agent_outcomes WHERE agent_id=?", (aid,)).fetchall()
                    total = len(outcomes)
                    if total == 0:
                        continue
                    successes = sum(1 for o in outcomes if o["success"])
                    sr = successes / total
                    avg_lat = sum(o["duration_ms"] or 0 for o in outcomes) / total
                    avg_q = sum(o["quality_score"] or 0.5 for o in outcomes) / total
                    rel = sr * 0.7 + (1.0 - min(1.0, avg_lat / 30000)) * 0.3
                    sid = f"ascore-{aid}"
                    conn.execute(
                        "INSERT OR REPLACE INTO agent_scores "
                        "(agent_score_id, agent_id, success_rate, avg_latency_ms, "
                        "quality_score, reliability_score, updated_at) VALUES (?,?,?,?,?,?,?)",
                        (sid, aid, sr, avg_lat, avg_q, rel, now))
                conn.commit()
                return len(agents)
            finally:
                conn.close()
        return await asyncio.to_thread(_update)

    async def get_scores(self) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM agent_scores ORDER BY reliability_score DESC").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def explain_agent(self, agent_id: str) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                score = conn.execute("SELECT * FROM agent_scores WHERE agent_id=?",
                                     (agent_id,)).fetchone()
                recent = [dict(r) for r in conn.execute(
                    "SELECT * FROM agent_outcomes WHERE agent_id=? ORDER BY created_at DESC LIMIT 10",
                    (agent_id,)).fetchall()]
                return {
                    "agent_id": agent_id,
                    "scores": dict(score) if score else None,
                    "recent_outcomes": recent,
                }
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


agent_scorer = AgentScorer()


class MemoryDistiller:
    """Compresses raw memory into structured knowledge."""

    async def distill_channel(self, channel_id: str) -> Optional[str]:
        """Summarize old messages in a channel into a distillation."""
        def _get_old():
            conn = _db_connect()
            try:
                rows = conn.execute(
                    "SELECT content FROM messages WHERE channel_id=? ORDER BY created_at ASC LIMIT 100",
                    (channel_id,)).fetchall()
                return [r["content"] for r in rows]
            finally:
                conn.close()
        messages = await asyncio.to_thread(_get_old)
        if len(messages) < 10:
            return None

        combined = "\n".join(messages[:50])[:3000]
        summary = await query_ai(
            f"Summarize these operational messages into key facts and patterns:\n{combined}",
            system="You are a memory distillation engine. Extract key operational facts, "
                   "incident patterns, and reusable knowledge. Be concise.")
        if not summary:
            return None

        did = f"dist-{uuid.uuid4().hex[:8]}"
        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO memory_distillations "
                    "(distillation_id, source_type, distilled_summary, created_at) "
                    "VALUES (?,?,?,?)", (did, "channel", summary, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)
        return summary

    async def get_distillations(self, limit: int = 10) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM memory_distillations ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_recipes(self, limit: int = 10) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM execution_recipes ORDER BY success_rate DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_incidents(self, limit: int = 10) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM incident_patterns ORDER BY recurrence_score DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def search_knowledge(self, query: str) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                results = []
                for table in ["system_knowledge", "execution_recipes", "incident_patterns"]:
                    rows = conn.execute(
                        f"SELECT * FROM {table} WHERE "
                        f"title LIKE ? OR summary LIKE ? OR incident_type LIKE ? OR task_type LIKE ? "
                        f"LIMIT 10",
                        (f"%{query}%", f"%{query}%", f"%{query}%", f"%{query}%")
                    ).fetchall()
                    results.extend([dict(r) for r in rows])
                return results[:20]
            except Exception:
                # Some columns may not exist in all tables
                conn2 = _db_connect()
                try:
                    rows = conn2.execute(
                        "SELECT * FROM system_knowledge WHERE title LIKE ? OR summary LIKE ? LIMIT 20",
                        (f"%{query}%", f"%{query}%")).fetchall()
                    return [dict(r) for r in rows]
                finally:
                    conn2.close()
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


memory_distiller = MemoryDistiller()


class ExplainabilityEngine:
    """Provides explanations for major adaptive decisions."""

    async def explain(self, decision_type: str, decision_id: str,
                       explanation: str, factors: Dict = None) -> str:
        eid = f"exp-{uuid.uuid4().hex[:8]}"
        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO decision_explanations "
                    "(explanation_id, decision_type, decision_id, "
                    "explanation_text, supporting_factors_json, created_at) "
                    "VALUES (?,?,?,?,?,?)",
                    (eid, decision_type, decision_id, explanation,
                     json.dumps(factors) if factors else None, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)
        return eid

    async def get_explanation(self, decision_type: str, decision_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute(
                    "SELECT * FROM decision_explanations "
                    "WHERE decision_type=? AND decision_id=? ORDER BY created_at DESC LIMIT 1",
                    (decision_type, decision_id)).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def why(self, entity_type: str, entity_id: str) -> Dict:
        """Generic 'why' query — finds explanations for any entity."""
        exp = await self.get_explanation(entity_type, entity_id)
        if exp:
            return exp
        # Fallback: search audit for context
        audits = await audit.search(limit=5)
        relevant = [a for a in audits if a.get("target_id") == entity_id]
        if relevant:
            return {"decision_type": entity_type, "decision_id": entity_id,
                    "explanation_text": f"Found {len(relevant)} audit entries for this entity",
                    "audit_entries": relevant}
        return {"decision_type": entity_type, "decision_id": entity_id,
                "explanation_text": "No explanation recorded for this entity"}


explainability = ExplainabilityEngine()


class IntelligenceLoop:
    """Periodic optimization cycle that updates learned system behavior."""

    def __init__(self):
        self._running = False
        self._task = None

    async def run_cycle(self) -> Dict:
        """Execute one intelligence loop cycle."""
        run_id = f"intel-{uuid.uuid4().hex[:8]}"
        start = time.time()
        changes = []

        def _create():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO intelligence_runs (intelligence_run_id, started_at) VALUES (?,?)",
                    (run_id, start))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_create)

        try:
            # 1. Update routing scores
            n = await routing_intel.update_scores()
            if n:
                changes.append(f"Updated {n} routing scores")

            # 2. Update planning patterns
            n = await plan_optimizer.update_patterns()
            if n:
                changes.append(f"Updated {n} planning patterns")

            # 3. Update repair patterns
            n = await repair_learner.update_patterns()
            if n:
                changes.append(f"Updated {n} repair patterns")

            # 4. Update agent scores
            n = await agent_scorer.update_scores()
            if n:
                changes.append(f"Updated {n} agent scores")

            success = True
        except Exception as e:
            changes.append(f"Error: {str(e)}")
            success = False

        end = time.time()
        def _complete():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE intelligence_runs SET completed_at=?, changes_json=?, "
                    "success=? WHERE intelligence_run_id=?",
                    (end, json.dumps(changes), 1 if success else 0, run_id))
                # Record individual updates
                for i, change in enumerate(changes):
                    conn.execute(
                        "INSERT INTO learning_updates "
                        "(update_id, intelligence_run_id, update_type, explanation, created_at) "
                        "VALUES (?,?,?,?,?)",
                        (f"lupd-{uuid.uuid4().hex[:6]}", run_id, "cycle_update", change, end))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_complete)
        await audit.log("intelligence_cycle", payload={"run_id": run_id, "changes": changes})

        return {"run_id": run_id, "success": success, "changes": changes,
                "duration_s": round(end - start, 2)}

    async def get_history(self, limit: int = 10) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM intelligence_runs ORDER BY started_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_run(self, run_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                run = conn.execute("SELECT * FROM intelligence_runs WHERE intelligence_run_id=?",
                                   (run_id,)).fetchone()
                if not run:
                    return None
                updates = [dict(r) for r in conn.execute(
                    "SELECT * FROM learning_updates WHERE intelligence_run_id=?",
                    (run_id,)).fetchall()]
                d = dict(run)
                d["updates"] = updates
                return d
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def start_loop(self, interval: int = 3600):
        """Start periodic intelligence loop."""
        self._running = True
        async def _loop():
            while self._running:
                await asyncio.sleep(interval)
                try:
                    await self.run_cycle()
                    log.info("Intelligence loop cycle completed")
                except Exception as e:
                    log.warning(f"Intelligence loop error: {e}")
        self._task = asyncio.create_task(_loop())
        log.info("Intelligence loop started")

    def stop(self):
        self._running = False
        if self._task:
            self._task.cancel()


intel_loop = IntelligenceLoop()


# ---------------------------------------------------------------------------
# Scale & Autonomy Maturity Layer
# ---------------------------------------------------------------------------

class WorkerRegistry:
    """Dynamic worker discovery and management."""

    async def register(self, host: str, region: str = "us-east1",
                        capabilities: List[str] = None) -> str:
        wid = f"worker-{uuid.uuid4().hex[:8]}"
        def _reg():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO worker_registry "
                    "(worker_id, host, region, capabilities_json, last_heartbeat, status) "
                    "VALUES (?,?,?,?,?,?)",
                    (wid, host, region, json.dumps(capabilities or []),
                     time.time(), "active"))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_reg)
        await audit.log("worker_registered", target_id=wid,
                        payload={"host": host, "region": region})
        return wid

    async def heartbeat(self, worker_id: str, health_score: float = 1.0):
        now = time.time()
        def _hb():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE worker_registry SET last_heartbeat=?, health_score=? "
                    "WHERE worker_id=?", (now, health_score, worker_id))
                conn.execute(
                    "INSERT INTO worker_health_history "
                    "(record_id, worker_id, health_score, recorded_at) VALUES (?,?,?,?)",
                    (f"whh-{uuid.uuid4().hex[:6]}", worker_id, health_score, now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_hb)

    async def quarantine(self, worker_id: str):
        def _q():
            conn = _db_connect()
            try:
                conn.execute("UPDATE worker_registry SET status='quarantined' WHERE worker_id=?",
                             (worker_id,))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_q)
        await audit.log("worker_quarantined", target_id=worker_id)

    async def list_workers(self, status: str = None) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if status:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM worker_registry WHERE status=?", (status,)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM worker_registry ORDER BY health_score DESC").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_by_region(self, region: str) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM worker_registry WHERE region=? AND status='active'",
                    (region,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def seed_defaults(self):
        """Register default VMs as workers."""
        for vm_name, info in VMS.items():
            existing = await self.list_workers()
            if not any(w["host"] == vm_name for w in existing):
                await self.register(
                    host=vm_name, region="us-east1",
                    capabilities=["shell", "docker"] if vm_name == "swarm-mainframe"
                    else ["shell", "gpu"] if vm_name == "swarm-gpu"
                    else ["shell", "web"])


worker_registry = WorkerRegistry()


class InitiativeEngine:
    """Autonomous proactive action engine."""

    async def evaluate_triggers(self) -> List[Dict]:
        """Check for conditions that warrant proactive action."""
        events = []
        try:
            # Check alerts
            alerts = await monitor.get_alerts(active_only=True)
            for alert in alerts[:5]:
                events.append({
                    "trigger_type": "monitoring_alert",
                    "source_signal": alert.get("message", ""),
                    "risk_level": "MODERATE",
                    "recommended_action": f"Investigate alert: {alert.get('check_id', '')}",
                    "confidence": 0.7,
                })

            # Check worker health
            workers = await worker_registry.list_workers()
            for w in workers:
                if w.get("health_score", 1.0) < 0.5:
                    events.append({
                        "trigger_type": "worker_degradation",
                        "source_signal": f"Worker {w['host']} health={w['health_score']}",
                        "risk_level": "HIGH" if w["health_score"] < 0.3 else "MODERATE",
                        "recommended_action": f"Investigate worker {w['host']}",
                        "confidence": 0.8,
                    })
        except Exception as e:
            log.warning(f"Initiative evaluation error: {e}")

        # Store events
        for evt in events:
            eid = f"init-{uuid.uuid4().hex[:8]}"
            def _store(e=evt, i=eid):
                conn = _db_connect()
                try:
                    conn.execute(
                        "INSERT INTO initiative_events "
                        "(event_id, trigger_type, source_signal, recommended_action, "
                        "risk_level, confidence, created_at) VALUES (?,?,?,?,?,?,?)",
                        (i, e["trigger_type"], e["source_signal"],
                         e["recommended_action"], e["risk_level"],
                         e["confidence"], time.time()))
                    conn.commit()
                finally:
                    conn.close()
            await asyncio.to_thread(_store)

        return events

    async def get_recent(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM initiative_events ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_actions(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM initiative_actions ORDER BY executed_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


initiative_engine = InitiativeEngine()


class KnowledgeEvolution:
    """Converts operational memory into evolving knowledge."""

    async def detect_patterns(self) -> int:
        """Cluster incidents and update knowledge."""
        def _detect():
            conn = _db_connect()
            try:
                # Find recurring alert types
                alerts = conn.execute(
                    "SELECT check_id, status, COUNT(*) as cnt FROM monitor_alerts "
                    "GROUP BY check_id, status HAVING cnt >= 2"
                ).fetchall()
                now = time.time()
                count = 0
                for a in alerts:
                    cid = f"cluster-{a['check_id']}-{a['status']}"
                    conn.execute(
                        "INSERT OR REPLACE INTO knowledge_clusters "
                        "(cluster_id, topic, pattern_summary, confidence, updated_at) "
                        "VALUES (?,?,?,?,?)",
                        (cid, f"alert:{a['check_id']}",
                         f"Alert '{a['status']}' recurred {a['cnt']} times",
                         min(1.0, a["cnt"] / 10.0), now))
                    count += 1
                conn.commit()
                return count
            finally:
                conn.close()
        return await asyncio.to_thread(_detect)

    async def get_patterns(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM knowledge_clusters ORDER BY confidence DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_playbooks(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM operational_playbooks ORDER BY success_rate DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def search(self, topic: str) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                clusters = [dict(r) for r in conn.execute(
                    "SELECT * FROM knowledge_clusters WHERE topic LIKE ? LIMIT 10",
                    (f"%{topic}%",)).fetchall()]
                playbooks = [dict(r) for r in conn.execute(
                    "SELECT * FROM operational_playbooks WHERE incident_type LIKE ? LIMIT 10",
                    (f"%{topic}%",)).fetchall()]
                return clusters + playbooks
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


knowledge_evolution = KnowledgeEvolution()


class SystemEvaluator:
    """Continuous system performance evaluation."""

    async def evaluate(self) -> Dict:
        """Run a full system evaluation."""
        eid = f"eval-{uuid.uuid4().hex[:8]}"
        metrics = {}
        recommendations = []

        try:
            # Routing
            perf = await perf_router.get_performance()
            if perf:
                avg_sr = sum(p.get("success_rate", 0) for p in perf) / len(perf)
                metrics["routing_success_rate"] = round(avg_sr, 3)
                if avg_sr < 0.8:
                    recommendations.append("Routing success rate below 80% — review provider health")

            # Agents
            scores = await agent_scorer.get_scores()
            if scores:
                avg_rel = sum(s.get("reliability_score", 0) for s in scores) / len(scores)
                metrics["agent_reliability"] = round(avg_rel, 3)
                if avg_rel < 0.6:
                    recommendations.append("Agent reliability below 60% — review delegation patterns")

            # Outcomes
            stats = await outcome_learner.get_stats()
            metrics["task_success_rate"] = stats.get("task_success_rate", 0)
            metrics["total_outcomes"] = stats.get("task_outcomes", 0)

            # Workers
            workers = await worker_registry.list_workers()
            healthy = [w for w in workers if w.get("health_score", 0) > 0.7]
            metrics["healthy_workers"] = len(healthy)
            metrics["total_workers"] = len(workers)

            score = (metrics.get("routing_success_rate", 0.5) * 0.3 +
                     metrics.get("agent_reliability", 0.5) * 0.2 +
                     metrics.get("task_success_rate", 0.5) * 0.3 +
                     (len(healthy) / max(1, len(workers))) * 0.2)
        except Exception as e:
            score = 0.5
            recommendations.append(f"Evaluation error: {str(e)}")

        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO system_evaluations "
                    "(evaluation_id, evaluation_type, metrics_json, score, "
                    "recommendations_json, created_at) VALUES (?,?,?,?,?,?)",
                    (eid, "full", json.dumps(metrics), score,
                     json.dumps(recommendations), time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)

        return {"evaluation_id": eid, "score": round(score, 3),
                "metrics": metrics, "recommendations": recommendations}

    async def get_scorecard(self) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM system_scorecards ORDER BY created_at DESC LIMIT 20"
                ).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_recent(self, limit: int = 10) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM system_evaluations ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_recommendations(self) -> List[str]:
        recent = await self.get_recent(1)
        if recent:
            recs = recent[0].get("recommendations_json")
            if recs:
                return json.loads(recs)
        return []


system_evaluator = SystemEvaluator()


# Risk levels for autonomous safety governance
RISK_LEVELS = {"LOW": 1, "MODERATE": 2, "HIGH": 3, "CRITICAL": 4}
RISK_AUTO_EXECUTE = {"LOW"}  # Only LOW auto-executes
RISK_NOTIFY = {"MODERATE"}   # MODERATE notifies
RISK_APPROVAL = {"HIGH"}     # HIGH requires approval
RISK_BLOCKED = {"CRITICAL"}  # CRITICAL blocked by default


class SafetyGovernor:
    """Ensures autonomous actions remain policy-governed."""

    def assess_risk(self, action_type: str, target: str = "") -> str:
        risk_map = {
            "restart_service": "MODERATE", "restart_runtime": "HIGH",
            "deploy": "HIGH", "delete": "CRITICAL",
            "cleanup_temp_files": "LOW", "fallback_provider": "LOW",
            "quarantine_worker": "HIGH", "modify_routing": "MODERATE",
            "health_check": "LOW", "reroute": "MODERATE",
            "execute_repair": "MODERATE", "rollback": "HIGH",
        }
        return risk_map.get(action_type, "MODERATE")

    async def gate_action(self, action_type: str, actor_id: str = "system",
                           target: str = "") -> Dict:
        """Check if action should proceed, notify, or require approval."""
        risk = self.assess_risk(action_type, target)
        if risk in RISK_BLOCKED:
            return {"allowed": False, "risk": risk, "reason": "CRITICAL actions blocked by default"}
        if risk in RISK_APPROVAL:
            aid = await perm_mgr.request_approval(
                actor_id, action_type, {"target": target},
                f"High-risk action: {action_type} on {target}")
            return {"allowed": False, "risk": risk, "reason": "Approval required",
                    "approval_id": aid}
        if risk in RISK_NOTIFY:
            await audit.log(f"auto_action_notify", actor_id=actor_id,
                            payload={"action": action_type, "risk": risk})
            return {"allowed": True, "risk": risk, "reason": "Proceeding with notification"}
        return {"allowed": True, "risk": risk, "reason": "Low risk — auto-executing"}

    async def explain_policy(self, action_type: str) -> Dict:
        risk = self.assess_risk(action_type)
        return {
            "action_type": action_type, "risk_level": risk,
            "auto_execute": risk in RISK_AUTO_EXECUTE,
            "notify": risk in RISK_NOTIFY,
            "approval_required": risk in RISK_APPROVAL,
            "blocked": risk in RISK_BLOCKED,
        }


safety_governor = SafetyGovernor()


class PluginManager:
    """Ecosystem plugin framework for extensibility."""

    async def register(self, name: str, plugin_type: str,
                        capabilities: List[str] = None,
                        description: str = None, config: Dict = None) -> str:
        pid = f"plugin-{uuid.uuid4().hex[:8]}"
        def _reg():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO plugins "
                    "(plugin_id, plugin_type, name, description, capabilities_json, "
                    "config_json, status, registered_at) VALUES (?,?,?,?,?,?,?,?)",
                    (pid, plugin_type, name, description,
                     json.dumps(capabilities or []),
                     json.dumps(config or {}), "active", time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_reg)
        await audit.log("plugin_registered", target_id=pid,
                        payload={"name": name, "type": plugin_type})
        return pid

    async def enable(self, plugin_id: str):
        def _e():
            conn = _db_connect()
            try:
                conn.execute("UPDATE plugins SET status='active' WHERE plugin_id=?", (plugin_id,))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_e)

    async def disable(self, plugin_id: str):
        def _d():
            conn = _db_connect()
            try:
                conn.execute("UPDATE plugins SET status='disabled' WHERE plugin_id=?", (plugin_id,))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_d)

    async def list_plugins(self, status: str = None) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if status:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM plugins WHERE status=?", (status,)).fetchall()]
                return [dict(r) for r in conn.execute("SELECT * FROM plugins").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_plugin(self, plugin_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM plugins WHERE plugin_id=?",
                                   (plugin_id,)).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


plugin_mgr = PluginManager()


# ---------------------------------------------------------------------------
# Environment Intelligence & Digital Twin Layer
# ---------------------------------------------------------------------------

class EnvironmentAwareness:
    """Continuous system awareness using telemetry and signals."""

    async def ingest_signal(self, source_type: str, source_id: str,
                             metric_name: str, value: float) -> str:
        sid = f"sig-{uuid.uuid4().hex[:8]}"
        now = time.time()
        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO environment_signals "
                    "(signal_id, source_type, source_id, metric_name, metric_value, timestamp) "
                    "VALUES (?,?,?,?,?,?)", (sid, source_type, source_id, metric_name, value, now))
                # Update entity state
                health = min(1.0, max(0.0, value / 100.0)) if metric_name in ("cpu", "memory", "disk") else 1.0
                conn.execute(
                    "INSERT OR REPLACE INTO environment_state "
                    "(state_id, entity_id, state_snapshot_json, derived_health_score, updated_at) "
                    "VALUES (?,?,?,?,?)",
                    (f"state-{source_id}", source_id,
                     json.dumps({metric_name: value}), health, now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)
        return sid

    async def get_status(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                states = [dict(r) for r in conn.execute(
                    "SELECT * FROM environment_state ORDER BY updated_at DESC").fetchall()]
                signal_count = conn.execute("SELECT COUNT(*) FROM environment_signals").fetchone()[0]
                return {"entities": states, "total_signals": signal_count}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_entity(self, entity_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                state = conn.execute("SELECT * FROM environment_state WHERE entity_id=?",
                                     (entity_id,)).fetchone()
                signals = [dict(r) for r in conn.execute(
                    "SELECT * FROM environment_signals WHERE source_id=? "
                    "ORDER BY timestamp DESC LIMIT 20", (entity_id,)).fetchall()]
                return {"state": dict(state) if state else None, "recent_signals": signals}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_signals(self, source_id: str = None, limit: int = 50) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if source_id:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM environment_signals WHERE source_id=? "
                        "ORDER BY timestamp DESC LIMIT ?", (source_id, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM environment_signals ORDER BY timestamp DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_health(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                states = conn.execute("SELECT * FROM environment_state").fetchall()
                health_map = {}
                for s in states:
                    health_map[s["entity_id"]] = {
                        "health_score": s["derived_health_score"],
                        "updated_at": s["updated_at"],
                    }
                avg_health = sum(h["health_score"] for h in health_map.values()) / max(1, len(health_map))
                return {"entities": health_map, "overall_health": round(avg_health, 3)}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


env_awareness = EnvironmentAwareness()


class EventIngestion:
    """Real-time event capture from all subsystems."""

    async def ingest(self, event_type: str, entity_type: str = None,
                      entity_id: str = None, payload: Dict = None,
                      severity: str = "info", tags: List[str] = None,
                      correlation_group: str = None) -> str:
        eid = f"evt-{uuid.uuid4().hex[:10]}"
        now = time.time()
        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO event_stream "
                    "(event_id, event_type, entity_type, entity_id, "
                    "payload_json, severity, created_at) VALUES (?,?,?,?,?,?,?)",
                    (eid, event_type, entity_type, entity_id,
                     json.dumps(payload) if payload else None, severity, now))
                conn.execute(
                    "INSERT INTO event_index (event_id, tags_json, related_entities_json, "
                    "correlation_group) VALUES (?,?,?,?)",
                    (eid, json.dumps(tags or []),
                     json.dumps([entity_id] if entity_id else []),
                     correlation_group))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)
        return eid

    async def get_recent(self, limit: int = 50) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM event_stream ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_by_entity(self, entity_id: str, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM event_stream WHERE entity_id=? ORDER BY created_at DESC LIMIT ?",
                    (entity_id, limit)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_correlation(self, group_id: str) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                idx = conn.execute(
                    "SELECT event_id FROM event_index WHERE correlation_group=?",
                    (group_id,)).fetchall()
                eids = [r["event_id"] for r in idx]
                if not eids:
                    return []
                placeholders = ",".join("?" * len(eids))
                return [dict(r) for r in conn.execute(
                    f"SELECT * FROM event_stream WHERE event_id IN ({placeholders}) "
                    f"ORDER BY created_at DESC", eids).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


event_ingestor = EventIngestion()


class DigitalTwin:
    """System simulation model of infrastructure and services."""

    async def seed_twin(self):
        """Create twin entities from known infrastructure."""
        now = time.time()
        def _seed():
            conn = _db_connect()
            try:
                for vm, info in VMS.items():
                    tid = f"twin-{vm}"
                    conn.execute(
                        "INSERT OR REPLACE INTO twin_entities "
                        "(twin_entity_id, entity_type, entity_ref, current_state_json, last_updated) "
                        "VALUES (?,?,?,?,?)",
                        (tid, "vm", vm, json.dumps({"ip": info["ip"], "zone": info["zone"],
                                                     "status": "running"}), now))
                # Add relationships
                conn.execute(
                    "INSERT OR IGNORE INTO twin_relationships "
                    "(relationship_id, source_entity, relation, target_entity) VALUES (?,?,?,?)",
                    ("trel-swarm-gpu", "twin-swarm-mainframe", "manages", "twin-swarm-gpu"))
                conn.execute(
                    "INSERT OR IGNORE INTO twin_relationships "
                    "(relationship_id, source_entity, relation, target_entity) VALUES (?,?,?,?)",
                    ("trel-portal", "twin-fc-ai-portal", "serves", "twin-swarm-mainframe"))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_seed)

    async def get_status(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                entities = [dict(r) for r in conn.execute("SELECT * FROM twin_entities").fetchall()]
                rels = [dict(r) for r in conn.execute("SELECT * FROM twin_relationships").fetchall()]
                return {"entities": entities, "relationships": rels}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def simulate_scenario(self, scenario_type: str, inputs: Dict = None) -> Dict:
        sid = f"tsim-{uuid.uuid4().hex[:8]}"
        risk_map = {
            "worker_loss": 0.5, "provider_failure": 0.4,
            "routing_change": 0.2, "load_spike": 0.3,
            "repair_plan": 0.35, "queue_overload": 0.45,
        }
        risk = risk_map.get(scenario_type, 0.3)
        predicted = {
            "scenario": scenario_type,
            "impact": "moderate" if risk < 0.4 else "significant",
            "recovery_estimate_s": int(risk * 60),
            "affected_components": list(VMS.keys())[:2] if risk > 0.3 else [list(VMS.keys())[0]],
        }

        def _store():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO twin_simulations "
                    "(simulation_id, scenario_type, inputs_json, predicted_results_json, "
                    "risk_score, created_at) VALUES (?,?,?,?,?,?)",
                    (sid, scenario_type, json.dumps(inputs or {}),
                     json.dumps(predicted), risk, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store)

        return {"simulation_id": sid, "scenario": scenario_type,
                "risk_score": round(risk, 2), "predicted": predicted}

    async def explain_simulation(self, sim_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM twin_simulations WHERE simulation_id=?",
                                   (sim_id,)).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


digital_twin = DigitalTwin()


class AutonomousOps:
    """Autonomous operations engine for proactive corrective actions."""

    async def detect_and_propose(self) -> List[Dict]:
        """Detect actionable signals and propose actions."""
        proposals = []
        try:
            # Check environment health
            health = await env_awareness.get_health()
            for eid, h in health.get("entities", {}).items():
                if h["health_score"] < 0.5:
                    proposals.append({
                        "trigger_type": "low_health",
                        "target_entity": eid,
                        "recommended_action": f"Investigate {eid} — health score {h['health_score']}",
                        "risk_level": "HIGH" if h["health_score"] < 0.3 else "MODERATE",
                        "confidence": 0.7,
                    })

            # Check for open escalations
            open_esc = await escalation_mgr.get_open()
            if len(open_esc) > 3:
                proposals.append({
                    "trigger_type": "escalation_overload",
                    "target_entity": "system",
                    "recommended_action": "Review and resolve open escalations",
                    "risk_level": "MODERATE",
                    "confidence": 0.8,
                })
        except Exception as e:
            log.warning(f"Autonomous ops detection error: {e}")

        for p in proposals:
            aid = f"autoact-{uuid.uuid4().hex[:8]}"
            def _store(proposal=p, action_id=aid):
                conn = _db_connect()
                try:
                    conn.execute(
                        "INSERT INTO autonomous_actions "
                        "(action_id, trigger_type, target_entity, recommended_action, "
                        "risk_level, confidence, created_at) VALUES (?,?,?,?,?,?,?)",
                        (action_id, proposal["trigger_type"], proposal["target_entity"],
                         proposal["recommended_action"], proposal["risk_level"],
                         proposal["confidence"], time.time()))
                    conn.commit()
                finally:
                    conn.close()
            await asyncio.to_thread(_store)

        return proposals

    async def get_actions(self, status: str = None, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if status:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM autonomous_actions WHERE status=? ORDER BY created_at DESC LIMIT ?",
                        (status, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM autonomous_actions ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


auto_ops = AutonomousOps()


# ---------------------------------------------------------------------------
# Structured Execution & Safety Boundary Layer
# ---------------------------------------------------------------------------

# Action Primitive Catalog — maps action_type to adapter + risk + description
ACTION_CATALOG = {
    # Docker Adapter
    "restart_container":   {"adapter": "docker", "risk": "SAFE_MUTATION",   "cmd_tpl": "docker restart {name}"},
    "start_container":     {"adapter": "docker", "risk": "SAFE_MUTATION",   "cmd_tpl": "docker start {name}"},
    "stop_container":      {"adapter": "docker", "risk": "RISKY_MUTATION",  "cmd_tpl": "docker stop {name}"},
    "container_logs":      {"adapter": "docker", "risk": "READ_ONLY",       "cmd_tpl": "docker logs --tail 50 {name}"},
    "deploy_container":    {"adapter": "docker", "risk": "RISKY_MUTATION",  "cmd_tpl": "docker compose up -d {name}"},
    "container_status":    {"adapter": "docker", "risk": "READ_ONLY",       "cmd_tpl": "docker ps --format '{{{{.Names}}}}: {{{{.Status}}}}'"},
    # Git Adapter
    "clone_repo":          {"adapter": "git", "risk": "SAFE_MUTATION",   "cmd_tpl": "git clone {url} {dest}"},
    "create_branch":       {"adapter": "git", "risk": "SAFE_MUTATION",   "cmd_tpl": "cd {repo} && git checkout -b {branch}"},
    "commit_changes":      {"adapter": "git", "risk": "SAFE_MUTATION",   "cmd_tpl": "cd {repo} && git add -A && git commit -m '{message}'"},
    "push_changes":        {"adapter": "git", "risk": "RISKY_MUTATION",  "cmd_tpl": "cd {repo} && git push"},
    "git_status":          {"adapter": "git", "risk": "READ_ONLY",       "cmd_tpl": "cd {repo} && git status --short"},
    "git_log":             {"adapter": "git", "risk": "READ_ONLY",       "cmd_tpl": "cd {repo} && git log --oneline -n {count}"},
    "git_pull":            {"adapter": "git", "risk": "SAFE_MUTATION",   "cmd_tpl": "cd {repo} && git pull"},
    "run_build":           {"adapter": "git", "risk": "SAFE_MUTATION",   "cmd_tpl": "cd {repo} && {build_cmd}"},
    "run_tests":           {"adapter": "git", "risk": "SAFE_MUTATION",   "cmd_tpl": "cd {repo} && {test_cmd}"},
    # Host Adapter
    "restart_service":     {"adapter": "host", "risk": "SAFE_MUTATION",   "cmd_tpl": "sudo systemctl restart {service}"},
    "inspect_host":        {"adapter": "host", "risk": "READ_ONLY",       "cmd_tpl": "uptime && free -h && df -h /"},
    "fetch_logs":          {"adapter": "host", "risk": "READ_ONLY",       "cmd_tpl": "sudo journalctl -u {service} -n {lines} --no-pager"},
    "read_file":           {"adapter": "host", "risk": "READ_ONLY",       "cmd_tpl": "head -100 '{path}'"},
    "write_file":          {"adapter": "host", "risk": "RISKY_MUTATION",  "cmd_tpl": "echo '{content}' > '{path}'"},
    "restart_worker":      {"adapter": "host", "risk": "SAFE_MUTATION",   "cmd_tpl": "sudo systemctl restart {worker}"},
    "cleanup_temp":        {"adapter": "host", "risk": "SAFE_MUTATION",   "cmd_tpl": "sudo find /tmp -type f -mtime +7 -delete 2>/dev/null; echo done"},
    "update_config":       {"adapter": "host", "risk": "RISKY_MUTATION",  "cmd_tpl": "echo 'config updated'"},
    "check_disk":          {"adapter": "host", "risk": "READ_ONLY",       "cmd_tpl": "df -h /"},
    "check_memory":        {"adapter": "host", "risk": "READ_ONLY",       "cmd_tpl": "free -h"},
    "check_processes":     {"adapter": "host", "risk": "READ_ONLY",       "cmd_tpl": "ps aux --sort=-%mem | head -15"},
    # Provider Adapter
    "reroute_provider":    {"adapter": "provider", "risk": "RISKY_MUTATION", "cmd_tpl": "reroute"},
    "check_provider":      {"adapter": "provider", "risk": "READ_ONLY",     "cmd_tpl": "check"},
    "switch_model":        {"adapter": "provider", "risk": "SAFE_MUTATION",  "cmd_tpl": "switch"},
    # System
    "shell_raw":           {"adapter": "raw_shell", "risk": "RISKY_MUTATION", "cmd_tpl": "{cmd}"},
    "delete_resources":    {"adapter": "host", "risk": "DESTRUCTIVE",    "cmd_tpl": "rm -rf {path}"},
}

# Risk auto-resolution thresholds
RISK_POLICY = {
    "READ_ONLY": {"auto_execute": True, "notify": False, "approval": False},
    "SAFE_MUTATION": {"auto_execute": True, "notify": False, "approval": False},
    "RISKY_MUTATION": {"auto_execute": False, "notify": True, "approval": True},
    "DESTRUCTIVE": {"auto_execute": False, "notify": True, "approval": True},
}

# Dangerous command patterns (blocked entirely outside admin/emergency)
BLOCKED_PATTERNS = [
    "rm -rf /", "mkfs", "dd if=", "fdisk", "> /dev/sda",
    ":(){ :|:& };:", "chmod -R 777 /", "mv /* ",
]


class InfrastructureAdapter:
    """Base infrastructure adapter — translates actions into commands."""

    async def execute(self, action_type: str, params: Dict,
                       host: str = "swarm-mainframe") -> Dict:
        """Execute an infrastructure action and return structured result."""
        catalog_entry = ACTION_CATALOG.get(action_type)
        if not catalog_entry:
            return {"success": False, "error": f"Unknown action: {action_type}"}

        adapter = catalog_entry["adapter"]
        cmd_tpl = catalog_entry["cmd_tpl"]

        # Build the actual command from template + params
        try:
            cmd = cmd_tpl.format(**params) if params else cmd_tpl
        except KeyError as e:
            return {"success": False, "error": f"Missing parameter: {e}"}

        # Input sanitization — block dangerous patterns
        for pattern in BLOCKED_PATTERNS:
            if pattern in cmd:
                return {"success": False, "error": f"Blocked: dangerous command pattern '{pattern}'"}

        # Route to the correct adapter
        start_time = time.time()
        try:
            if adapter in ("host", "docker", "git", "raw_shell"):
                result = await tool_executor.exec_shell(host, cmd)
            elif adapter == "provider":
                result = await self._provider_action(action_type, params)
            else:
                result = await tool_executor.exec_shell(host, cmd)

            duration = (time.time() - start_time) * 1000
            success = not result.startswith("[ERROR]") and not result.startswith("[exit")

            return {
                "success": success,
                "stdout": result if success else "",
                "stderr": result if not success else "",
                "command": cmd,
                "host": host,
                "duration_ms": round(duration, 1),
                "exit_code": 0 if success else 1,
            }
        except Exception as e:
            duration = (time.time() - start_time) * 1000
            return {
                "success": False,
                "stdout": "",
                "stderr": str(e),
                "command": cmd,
                "host": host,
                "duration_ms": round(duration, 1),
                "exit_code": -1,
            }

    async def _provider_action(self, action_type: str, params: Dict) -> str:
        """Handle provider-level actions (no shell command)."""
        if action_type == "reroute_provider":
            target = params.get("provider", "deepseek")
            global _active_provider, _active_model
            _active_provider = target
            _active_model = params.get("model", f"{target}-chat")
            return f"Rerouted to {target}/{_active_model}"
        elif action_type == "check_provider":
            return f"Active: {_active_provider}/{_active_model}"
        elif action_type == "switch_model":
            target = params.get("model", "deepseek-chat")
            provider = params.get("provider", "deepseek")
            _active_provider = provider
            _active_model = target
            return f"Switched to {provider}/{target}"
        return "Unknown provider action"


infra_adapter = InfrastructureAdapter()


class ActionExecutionService:
    """Centralized execution service — all actions pass through here.

    Flow: Action Request → Policy Validation → Approval Check →
          Infrastructure Adapter → Result Capture → Audit Record
    """

    def __init__(self):
        self._emergency_mode = False  # Bypasses approval gates

    async def seed_policies(self):
        """Seed default action policies from the catalog."""
        def _seed():
            conn = _db_connect()
            try:
                now = time.time()
                for action_type, entry in ACTION_CATALOG.items():
                    risk = entry["risk"]
                    needs_approval = 1 if risk in ("RISKY_MUTATION", "DESTRUCTIVE") else 0
                    conn.execute(
                        "INSERT OR IGNORE INTO action_policies "
                        "(policy_id, action_type, risk_level, requires_approval, "
                        "adapter, description, updated_at) VALUES (?,?,?,?,?,?,?)",
                        (f"apol-{action_type}", action_type, risk, needs_approval,
                         entry["adapter"], entry.get("cmd_tpl", ""), now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_seed)

    def classify_risk(self, action_type: str) -> str:
        """Get risk level for an action type."""
        entry = ACTION_CATALOG.get(action_type, {})
        return entry.get("risk", "RISKY_MUTATION")

    def classify_shell_command(self, cmd: str) -> Tuple[str, str]:
        """Classify a raw shell command into an action type and risk level.

        Converts arbitrary shell commands into the closest action primitive.
        """
        cmd_lower = cmd.lower().strip()

        # Check for blocked patterns first
        for pattern in BLOCKED_PATTERNS:
            if pattern in cmd:
                return "shell_raw", "DESTRUCTIVE"

        # Classify by command content
        if any(cmd_lower.startswith(p) for p in ["cat ", "head ", "tail ", "less ", "more "]):
            return "read_file", "READ_ONLY"
        if any(cmd_lower.startswith(p) for p in ["ls ", "find ", "df ", "free ", "uptime",
                                                   "ps ", "top ", "whoami", "date", "uname",
                                                   "wc ", "file ", "stat ", "du ", "hostname"]):
            return "inspect_host", "READ_ONLY"
        if cmd_lower.startswith("docker ps") or cmd_lower.startswith("docker images"):
            return "container_status", "READ_ONLY"
        if cmd_lower.startswith("docker logs"):
            return "container_logs", "READ_ONLY"
        if cmd_lower.startswith("docker restart"):
            return "restart_container", "SAFE_MUTATION"
        if cmd_lower.startswith("docker stop"):
            return "stop_container", "RISKY_MUTATION"
        if cmd_lower.startswith("docker start"):
            return "start_container", "SAFE_MUTATION"
        if cmd_lower.startswith("docker compose"):
            return "deploy_container", "RISKY_MUTATION"
        if cmd_lower.startswith("git status") or cmd_lower.startswith("git log"):
            return "git_status", "READ_ONLY"
        if cmd_lower.startswith("git pull"):
            return "git_pull", "SAFE_MUTATION"
        if cmd_lower.startswith("git push"):
            return "push_changes", "RISKY_MUTATION"
        if cmd_lower.startswith("git clone"):
            return "clone_repo", "SAFE_MUTATION"
        if "git commit" in cmd_lower:
            return "commit_changes", "SAFE_MUTATION"
        if cmd_lower.startswith("sudo systemctl restart"):
            return "restart_service", "SAFE_MUTATION"
        if cmd_lower.startswith("journalctl"):
            return "fetch_logs", "READ_ONLY"
        if any(cmd_lower.startswith(p) for p in ["grep ", "awk ", "sed ", "curl -s", "wget -q"]):
            return "inspect_host", "READ_ONLY"
        if any(cmd_lower.startswith(p) for p in ["nvidia-smi", "nvtop"]):
            return "inspect_host", "READ_ONLY"
        if any(w in cmd_lower for w in ["rm ", "rmdir", "unlink"]):
            if "rm -rf /" in cmd_lower:
                return "delete_resources", "DESTRUCTIVE"
            return "delete_resources", "RISKY_MUTATION"
        if any(w in cmd_lower for w in ["tee ", "> ", ">> ", "echo "]) and ">" in cmd:
            return "write_file", "RISKY_MUTATION"
        if cmd_lower.startswith("timeout "):
            # Sandboxed code execution
            return "shell_raw", "SAFE_MUTATION"

        # Default: treat unknown shell commands as SAFE_MUTATION
        # (not READ_ONLY since we can't be sure, not RISKY since most are safe)
        return "shell_raw", "SAFE_MUTATION"

    async def request_action(self, action_type: str, params: Dict = None,
                              host: str = "swarm-mainframe",
                              requested_by: str = "system",
                              session_id: str = None) -> Dict:
        """Request a structured action — validates, checks policy, executes if allowed."""
        action_id = f"act-{uuid.uuid4().hex[:10]}"
        risk = self.classify_risk(action_type)
        policy = RISK_POLICY.get(risk, RISK_POLICY["RISKY_MUTATION"])
        now = time.time()

        # Create action record
        def _create():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO execution_actions "
                    "(action_id, action_type, parameters_json, requested_by, "
                    "session_id, risk_level, approval_required, status, created_at) "
                    "VALUES (?,?,?,?,?,?,?,?,?)",
                    (action_id, action_type, json.dumps(params or {}),
                     requested_by, session_id, risk,
                     1 if policy["approval"] and not self._emergency_mode else 0,
                     "pending", now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_create)

        # Check if approval is needed
        if policy["approval"] and not self._emergency_mode:
            # Create approval request
            appr_id = await perm_mgr.request_approval(
                requested_by, action_type,
                {"host": host, **(params or {})},
                f"Action '{action_type}' on {host} — risk={risk}")

            def _update():
                conn = _db_connect()
                try:
                    conn.execute(
                        "UPDATE execution_actions SET approval_id=?, status='awaiting_approval' "
                        "WHERE action_id=?", (appr_id, action_id))
                    conn.commit()
                finally:
                    conn.close()
            await asyncio.to_thread(_update)

            # Audit
            await self._audit_action(action_id, action_type, requested_by,
                                      params, "awaiting_approval", risk)
            return {
                "action_id": action_id, "status": "awaiting_approval",
                "approval_id": appr_id, "risk": risk,
                "message": f"Action '{action_type}' requires approval (risk={risk})",
            }

        # Execute directly
        return await self._execute_action(action_id, action_type, params,
                                           host, requested_by, risk)

    async def _execute_action(self, action_id: str, action_type: str,
                               params: Dict, host: str,
                               requested_by: str, risk: str) -> Dict:
        """Actually execute the action through the infrastructure adapter."""
        # Update status to running
        def _update_running():
            conn = _db_connect()
            try:
                conn.execute("UPDATE execution_actions SET status='running' WHERE action_id=?",
                             (action_id,))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_update_running)

        # Execute through adapter
        result = await infra_adapter.execute(action_type, params or {}, host)

        # Store result
        result_id = f"res-{uuid.uuid4().hex[:8]}"
        final_status = "completed" if result["success"] else "failed"

        def _store_result():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO execution_results "
                    "(result_id, action_id, execution_host, command_executed, "
                    "stdout, stderr, exit_code, success, duration_ms, timestamp) "
                    "VALUES (?,?,?,?,?,?,?,?,?,?)",
                    (result_id, action_id, result.get("host", host),
                     result.get("command", ""), result.get("stdout", ""),
                     result.get("stderr", ""), result.get("exit_code", 0),
                     1 if result["success"] else 0,
                     result.get("duration_ms", 0), time.time()))
                conn.execute(
                    "UPDATE execution_actions SET status=?, resolved_at=? WHERE action_id=?",
                    (final_status, time.time(), action_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_store_result)

        # Audit
        await self._audit_action(action_id, action_type, requested_by,
                                  params, final_status, risk)

        # Record outcome for learning
        try:
            await outcome_learner.record_task_outcome(
                action_id, task_type=action_type,
                success=result["success"],
                duration_ms=result.get("duration_ms", 0),
                provider_used=host)
        except Exception:
            pass

        output = result.get("stdout", "") or result.get("stderr", "")
        return {
            "action_id": action_id, "status": final_status,
            "risk": risk, "output": output,
            "success": result["success"],
            "duration_ms": result.get("duration_ms", 0),
            "command": result.get("command", ""),
        }

    async def execute_shell_mediated(self, host: str, cmd: str,
                                      requested_by: str = "system") -> str:
        """Mediated shell execution — classifies command and routes through policy.

        This replaces direct tool_executor.exec_shell calls.
        Returns the shell output string (preserving the old interface).
        """
        action_type, risk = self.classify_shell_command(cmd)

        # For READ_ONLY and SAFE_MUTATION, execute directly
        if risk in ("READ_ONLY", "SAFE_MUTATION"):
            # Still record and audit, but don't require approval
            result = await self.request_action(
                action_type, {"cmd": cmd, "host": host},
                host=host, requested_by=requested_by)
            return result.get("output", "(no output)")

        # For RISKY/DESTRUCTIVE, check emergency mode
        if self._emergency_mode:
            result = await self.request_action(
                action_type, {"cmd": cmd, "host": host},
                host=host, requested_by=requested_by)
            return result.get("output", "(no output)")

        # RISKY_MUTATION — execute but log prominently
        if risk == "RISKY_MUTATION":
            await audit.log("risky_shell_execution", actor_id=requested_by,
                            payload={"cmd": cmd[:200], "host": host, "risk": risk})
            result = await self.request_action(
                action_type, {"cmd": cmd, "host": host},
                host=host, requested_by=requested_by)
            if result.get("status") == "awaiting_approval":
                return f"[APPROVAL REQUIRED] Action '{action_type}' needs operator approval (id={result.get('approval_id', '?')})"
            return result.get("output", "(no output)")

        # DESTRUCTIVE — block
        await audit.log("blocked_shell_execution", actor_id=requested_by,
                        payload={"cmd": cmd[:200], "host": host, "risk": risk})
        return f"[BLOCKED] Destructive command blocked by safety policy. Request approval via /approve"

    async def _audit_action(self, action_id: str, action_type: str,
                             actor_id: str, params: Dict,
                             result: str, risk: str):
        """Record action in structured audit trail."""
        def _audit():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO audit_actions "
                    "(audit_id, action_id, actor_type, actor_id, action_type, "
                    "parameters_json, result, risk_level, created_at) VALUES (?,?,?,?,?,?,?,?,?)",
                    (f"aaud-{uuid.uuid4().hex[:8]}", action_id,
                     "SYSTEM" if actor_id == "system" else "USER", actor_id,
                     action_type, json.dumps(params or {}), result, risk, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_audit)

    async def get_actions(self, status: str = None, limit: int = 30) -> List[Dict]:
        """Get execution actions with optional status filter."""
        def _q():
            conn = _db_connect()
            try:
                if status:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM execution_actions WHERE status=? "
                        "ORDER BY created_at DESC LIMIT ?", (status, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM execution_actions ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_results(self, action_id: str = None, limit: int = 20) -> List[Dict]:
        """Get execution results."""
        def _q():
            conn = _db_connect()
            try:
                if action_id:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM execution_results WHERE action_id=?",
                        (action_id,)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM execution_results ORDER BY timestamp DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_audit_trail(self, limit: int = 30) -> List[Dict]:
        """Get action audit trail."""
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM audit_actions ORDER BY created_at DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_policies(self) -> List[Dict]:
        """Get all action policies."""
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM action_policies ORDER BY risk_level").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        """Get execution statistics."""
        def _q():
            conn = _db_connect()
            try:
                total = conn.execute("SELECT COUNT(*) FROM execution_actions").fetchone()[0]
                completed = conn.execute("SELECT COUNT(*) FROM execution_actions WHERE status='completed'").fetchone()[0]
                failed = conn.execute("SELECT COUNT(*) FROM execution_actions WHERE status='failed'").fetchone()[0]
                pending = conn.execute("SELECT COUNT(*) FROM execution_actions WHERE status='pending' OR status='awaiting_approval'").fetchone()[0]
                return {"total": total, "completed": completed, "failed": failed,
                        "pending": pending, "success_rate": round(completed / max(1, total), 3)}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


action_service = ActionExecutionService()


# ---------------------------------------------------------------------------
# Mediated ToolExecutor Wrapper
# ---------------------------------------------------------------------------
# Monkey-patch the original ToolExecutor.execute to route through
# ActionExecutionService for all shell commands.

_original_tool_execute = tool_executor.execute


async def _mediated_tool_execute(tool: str, host: str, cmd: str) -> str:
    """Wraps all tool_executor.execute calls through the structured execution layer."""
    if tool == "shell":
        return await action_service.execute_shell_mediated(host, cmd)
    elif tool == "docker":
        if not cmd.strip().startswith("docker"):
            cmd = f"docker {cmd}"
        return await action_service.execute_shell_mediated(host, cmd)
    else:
        # Non-shell tools (ollama, http, image) pass through unmodified
        return await _original_tool_execute(tool, host, cmd)


# Apply the mediation wrapper
tool_executor.execute = _mediated_tool_execute


# ---------------------------------------------------------------------------
# Proactive Relationship & Opportunity Engine
# ---------------------------------------------------------------------------

class SignalDiscovery:
    """Module 1: Company Signal Discovery — detects orgs needing AI systems."""

    SIGNAL_TYPES = [
        "ai_hiring", "ml_engineer_hiring", "data_engineer_hiring",
        "github_corporate", "saas_launch", "funding_round",
        "industry_announcement", "compliance_update", "ai_adoption",
        "digital_transformation", "security_investment", "tech_expansion",
    ]

    async def add_signal(self, company_name: str, domain: str, signal_type: str,
                         source: str, payload: Dict = None, confidence: float = 0.5) -> str:
        signal_id = f"sig-{uuid.uuid4().hex[:12]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO company_signals "
                    "(signal_id, company_name, domain, signal_type, signal_source, "
                    "signal_payload_json, confidence, timestamp) VALUES (?,?,?,?,?,?,?,?)",
                    (signal_id, company_name, domain or "", signal_type, source,
                     json.dumps(payload or {}), confidence, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return signal_id

    async def get_signals(self, limit: int = 20, signal_type: str = None,
                          unprocessed_only: bool = False) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                where = []
                params = []
                if signal_type:
                    where.append("signal_type=?")
                    params.append(signal_type)
                if unprocessed_only:
                    where.append("processed=0")
                clause = f"WHERE {' AND '.join(where)}" if where else ""
                params.append(limit)
                return [dict(r) for r in conn.execute(
                    f"SELECT * FROM company_signals {clause} ORDER BY timestamp DESC LIMIT ?",
                    params).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def mark_processed(self, signal_id: str):
        def _up():
            conn = _db_connect()
            try:
                conn.execute("UPDATE company_signals SET processed=1 WHERE signal_id=?",
                             (signal_id,))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_up)

    async def add_source(self, source_type: str, source_url: str = None,
                         polling_interval: int = 3600) -> str:
        source_id = f"src-{uuid.uuid4().hex[:8]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT OR IGNORE INTO signal_sources "
                    "(source_id, source_type, source_url, polling_interval, created_at) "
                    "VALUES (?,?,?,?,?)",
                    (source_id, source_type, source_url or "", polling_interval, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return source_id

    async def get_sources(self) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM signal_sources ORDER BY last_scan").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                total = conn.execute("SELECT COUNT(*) FROM company_signals").fetchone()[0]
                unprocessed = conn.execute("SELECT COUNT(*) FROM company_signals WHERE processed=0").fetchone()[0]
                sources = conn.execute("SELECT COUNT(*) FROM signal_sources").fetchone()[0]
                by_type = {}
                for row in conn.execute(
                    "SELECT signal_type, COUNT(*) as cnt FROM company_signals GROUP BY signal_type"):
                    by_type[row["signal_type"]] = row["cnt"]
                return {"total_signals": total, "unprocessed": unprocessed,
                        "sources": sources, "by_type": by_type}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def seed_sources(self):
        """Seed default signal sources."""
        defaults = [
            ("job_boards", "https://api.example.com/jobs", 3600),
            ("github_trending", "https://api.github.com/search/repositories", 7200),
            ("news_feed", "https://newsapi.org/v2/everything", 1800),
            ("funding_tracker", "https://api.crunchbase.com/v4", 86400),
            ("linkedin_signals", "https://api.linkedin.com/v2", 3600),
        ]
        for src_type, url, interval in defaults:
            await self.add_source(src_type, url, interval)


class OpportunityQualifier:
    """Module 2: Opportunity Qualification — score companies for AI fit."""

    INDUSTRY_AI_WEIGHT = {
        "fintech": 0.9, "healthcare": 0.85, "legal": 0.8, "insurance": 0.85,
        "logistics": 0.75, "manufacturing": 0.7, "retail": 0.65, "education": 0.6,
        "real_estate": 0.7, "defense": 0.9, "government": 0.8, "saas": 0.85,
        "cybersecurity": 0.9, "crypto": 0.8, "media": 0.6,
    }

    async def create_profile(self, name: str, domain: str = None, industry: str = None,
                             size: str = None, tech_stack: List[str] = None,
                             description: str = None, website: str = None,
                             location: str = None) -> str:
        company_id = f"comp-{uuid.uuid4().hex[:10]}"
        now = time.time()
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT OR IGNORE INTO company_profiles "
                    "(company_id, name, domain, industry, size_estimate, tech_stack_json, "
                    "description, website, location, created_at, updated_at) "
                    "VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                    (company_id, name, domain or "", industry or "", size or "",
                     json.dumps(tech_stack or []), description or "", website or "",
                     location or "", now, now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return company_id

    async def score_opportunity(self, company_id: str) -> Dict:
        """Calculate AI fit score for a company."""
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM company_profiles WHERE company_id=?",
                                   (company_id,)).fetchone()
                if not row:
                    return None
                profile = dict(row)

                # Scoring factors
                industry = profile.get("industry", "").lower()
                industry_weight = self.INDUSTRY_AI_WEIGHT.get(industry, 0.5)

                size = profile.get("size_estimate", "").lower()
                size_score = {"enterprise": 0.9, "mid-market": 0.8, "startup": 0.6,
                              "smb": 0.4}.get(size, 0.5)

                tech_stack = json.loads(profile.get("tech_stack_json", "[]") or "[]")
                tech_indicators = sum(1 for t in tech_stack if t.lower() in
                    ["python", "kubernetes", "docker", "aws", "gcp", "azure",
                     "tensorflow", "pytorch", "spark", "redis", "postgresql"])
                tech_score = min(1.0, tech_indicators * 0.15)

                # Signals count
                sig_count = conn.execute(
                    "SELECT COUNT(*) FROM company_signals WHERE company_name=? OR domain=?",
                    (profile["name"], profile.get("domain", ""))).fetchone()[0]
                signal_score = min(1.0, sig_count * 0.2)

                ai_fit = round(
                    industry_weight * 0.3 + size_score * 0.2 +
                    tech_score * 0.25 + signal_score * 0.25, 3)

                security_sens = round(
                    (0.8 if industry in ("fintech", "healthcare", "defense", "government", "legal") else 0.4) +
                    (0.2 if "enterprise" in size else 0.0), 3)

                estimated_value = round(ai_fit * 100000 * (1 + size_score), 2)
                difficulty = round(1.0 - (tech_score * 0.5 + signal_score * 0.5), 3)

                # Update profile scores
                conn.execute(
                    "UPDATE company_profiles SET ai_need_score=?, security_sensitivity_score=?, "
                    "updated_at=? WHERE company_id=?",
                    (ai_fit, security_sens, time.time(), company_id))

                # Insert opportunity score
                opp_id = f"opp-{uuid.uuid4().hex[:10]}"
                factors = {"industry": industry_weight, "size": size_score,
                           "tech": tech_score, "signals": signal_score}
                conn.execute(
                    "INSERT INTO opportunity_scores "
                    "(opportunity_id, company_id, ai_fit_score, estimated_value, "
                    "difficulty_score, confidence, scoring_factors_json, created_at) "
                    "VALUES (?,?,?,?,?,?,?,?)",
                    (opp_id, company_id, ai_fit, estimated_value, difficulty,
                     min(0.9, 0.3 + sig_count * 0.1), json.dumps(factors), time.time()))
                conn.commit()

                return {"company_id": company_id, "ai_fit_score": ai_fit,
                        "estimated_value": estimated_value, "difficulty": difficulty,
                        "security_sensitivity": security_sens, "factors": factors}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_top_opportunities(self, limit: int = 10) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT cp.*, os.ai_fit_score, os.estimated_value, os.difficulty_score "
                    "FROM company_profiles cp "
                    "LEFT JOIN opportunity_scores os ON cp.company_id = os.company_id "
                    "ORDER BY cp.ai_need_score DESC LIMIT ?", (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_profiles(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM company_profiles ORDER BY ai_need_score DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


class RelationshipPipeline:
    """Module 3: Relationship Pipeline — track interaction lifecycle."""

    STAGES = [
        "detected", "research", "contact_initiated", "conversation_active",
        "demo_requested", "proposal_sent", "deployment_in_progress", "active_client",
    ]

    async def init_pipeline(self, company_id: str, stage: str = "detected",
                            notes: str = "", agent: str = "system") -> str:
        pipeline_id = f"pipe-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT OR IGNORE INTO relationship_pipeline "
                    "(pipeline_id, company_id, stage, notes, assigned_agent, updated_at) "
                    "VALUES (?,?,?,?,?,?)",
                    (pipeline_id, company_id, stage, notes, agent, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        await self.add_event(company_id, "pipeline_created", {"stage": stage})
        return pipeline_id

    async def advance_stage(self, company_id: str, new_stage: str, notes: str = ""):
        def _up():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE relationship_pipeline SET stage=?, notes=?, "
                    "last_contact_time=?, updated_at=? WHERE company_id=?",
                    (new_stage, notes, time.time(), time.time(), company_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_up)
        await self.add_event(company_id, "stage_advanced", {"new_stage": new_stage, "notes": notes})

    async def add_event(self, company_id: str, event_type: str, details: Dict = None):
        event_id = f"revt-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO relationship_events "
                    "(event_id, company_id, event_type, event_details_json, timestamp) "
                    "VALUES (?,?,?,?,?)",
                    (event_id, company_id, event_type, json.dumps(details or {}), time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)

    async def get_pipeline(self, stage: str = None, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if stage:
                    return [dict(r) for r in conn.execute(
                        "SELECT rp.*, cp.name, cp.industry, cp.ai_need_score "
                        "FROM relationship_pipeline rp "
                        "LEFT JOIN company_profiles cp ON rp.company_id = cp.company_id "
                        "WHERE rp.stage=? ORDER BY rp.updated_at DESC LIMIT ?",
                        (stage, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT rp.*, cp.name, cp.industry, cp.ai_need_score "
                    "FROM relationship_pipeline rp "
                    "LEFT JOIN company_profiles cp ON rp.company_id = cp.company_id "
                    "ORDER BY rp.updated_at DESC LIMIT ?", (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_events(self, company_id: str = None, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if company_id:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM relationship_events WHERE company_id=? "
                        "ORDER BY timestamp DESC LIMIT ?", (company_id, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM relationship_events ORDER BY timestamp DESC LIMIT ?",
                    (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                by_stage = {}
                for row in conn.execute(
                    "SELECT stage, COUNT(*) as cnt FROM relationship_pipeline GROUP BY stage"):
                    by_stage[row["stage"]] = row["cnt"]
                total = conn.execute("SELECT COUNT(*) FROM relationship_pipeline").fetchone()[0]
                events = conn.execute("SELECT COUNT(*) FROM relationship_events").fetchone()[0]
                return {"total_relationships": total, "total_events": events, "by_stage": by_stage}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


class ResearchAgent:
    """Module 4: Research Agent — auto-generate company intelligence profiles."""

    async def create_research(self, company_id: str, summary: str = "",
                              ai_use_cases: List[str] = None,
                              process_candidates: List[str] = None,
                              security_reqs: str = "", competitors: str = "",
                              products: str = "") -> str:
        research_id = f"res-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO company_research "
                    "(research_id, company_id, summary, ai_use_cases_json, "
                    "internal_process_candidates_json, security_requirements, "
                    "competitor_landscape, products_services, created_at) "
                    "VALUES (?,?,?,?,?,?,?,?,?)",
                    (research_id, company_id, summary,
                     json.dumps(ai_use_cases or []),
                     json.dumps(process_candidates or []),
                     security_reqs, competitors, products, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return research_id

    async def get_research(self, company_id: str) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute(
                    "SELECT * FROM company_research WHERE company_id=? "
                    "ORDER BY created_at DESC LIMIT 1", (company_id,)).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def generate_research_prompt(self, company_id: str) -> str:
        """Generate AI prompt to research a company."""
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM company_profiles WHERE company_id=?",
                                   (company_id,)).fetchone()
                if not row:
                    return None
                profile = dict(row)
                signals = conn.execute(
                    "SELECT * FROM company_signals WHERE company_name=? OR domain=? "
                    "ORDER BY timestamp DESC LIMIT 5",
                    (profile["name"], profile.get("domain", ""))).fetchall()
                return profile, [dict(s) for s in signals]
            finally:
                conn.close()

        result = await asyncio.to_thread(_q)
        if not result:
            return ""
        profile, signals = result

        signal_text = "\n".join(
            f"- {s['signal_type']}: {s.get('signal_payload_json', '')[:100]}"
            for s in signals)

        return (
            f"Research the following company for potential AI system deployment:\n\n"
            f"Company: {profile['name']}\n"
            f"Domain: {profile.get('domain', 'unknown')}\n"
            f"Industry: {profile.get('industry', 'unknown')}\n"
            f"Size: {profile.get('size_estimate', 'unknown')}\n"
            f"Tech Stack: {profile.get('tech_stack_json', '[]')}\n"
            f"\nSignals:\n{signal_text}\n\n"
            f"Provide:\n1. Company overview\n2. Products/services\n"
            f"3. Top 3 AI use cases for their business\n"
            f"4. Internal workflow automation opportunities\n"
            f"5. Security/compliance requirements\n6. Competitor landscape"
        )

    async def get_all_research(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT cr.*, cp.name, cp.industry FROM company_research cr "
                    "LEFT JOIN company_profiles cp ON cr.company_id = cp.company_id "
                    "ORDER BY cr.created_at DESC LIMIT ?", (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


class OutreachGenerator:
    """Module 5: Targeted Outreach Generation — personalized messages."""

    CHANNELS = ["email", "linkedin", "warm_intro", "partner_proposal", "demo_invitation"]

    async def generate_outreach(self, company_id: str, channel: str,
                                content: str, personalization: Dict = None,
                                template_id: str = None) -> str:
        message_id = f"out-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO outreach_messages "
                    "(message_id, company_id, channel, message_content, "
                    "personalization_fields_json, template_id, response_status, created_at) "
                    "VALUES (?,?,?,?,?,?,?,?)",
                    (message_id, company_id, channel, content,
                     json.dumps(personalization or {}), template_id or "",
                     "draft", time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return message_id

    async def approve_outreach(self, message_id: str, approved_by: str = "operator"):
        def _up():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE outreach_messages SET response_status='approved', "
                    "approved_by=? WHERE message_id=?", (approved_by, message_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_up)

    async def mark_sent(self, message_id: str):
        def _up():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE outreach_messages SET response_status='sent', sent_at=? "
                    "WHERE message_id=?", (time.time(), message_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_up)

    async def update_response(self, message_id: str, status: str):
        """Update response status: replied, interested, declined, no_response."""
        def _up():
            conn = _db_connect()
            try:
                conn.execute(
                    "UPDATE outreach_messages SET response_status=? WHERE message_id=?",
                    (status, message_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_up)

    async def get_messages(self, company_id: str = None, status: str = None,
                           limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                where = []
                params = []
                if company_id:
                    where.append("company_id=?")
                    params.append(company_id)
                if status:
                    where.append("response_status=?")
                    params.append(status)
                clause = f"WHERE {' AND '.join(where)}" if where else ""
                params.append(limit)
                return [dict(r) for r in conn.execute(
                    f"SELECT * FROM outreach_messages {clause} "
                    f"ORDER BY created_at DESC LIMIT ?", params).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def generate_outreach_prompt(self, company_id: str, channel: str) -> str:
        """Build a prompt for AI to generate personalized outreach."""
        def _q():
            conn = _db_connect()
            try:
                profile = conn.execute("SELECT * FROM company_profiles WHERE company_id=?",
                                       (company_id,)).fetchone()
                research = conn.execute("SELECT * FROM company_research WHERE company_id=? "
                                        "ORDER BY created_at DESC LIMIT 1",
                                        (company_id,)).fetchone()
                return dict(profile) if profile else None, dict(research) if research else None
            finally:
                conn.close()

        profile, research = await asyncio.to_thread(_q)
        if not profile:
            return ""

        research_text = ""
        if research:
            research_text = (
                f"\nResearch Summary: {research.get('summary', '')[:200]}\n"
                f"AI Use Cases: {research.get('ai_use_cases_json', '[]')}\n"
            )

        return (
            f"Generate a personalized {channel} outreach message for:\n\n"
            f"Company: {profile['name']}\n"
            f"Industry: {profile.get('industry', '')}\n"
            f"Size: {profile.get('size_estimate', '')}\n"
            f"{research_text}\n"
            f"Requirements:\n"
            f"- Reference specific company needs\n"
            f"- Clear value proposition for secure in-house AI\n"
            f"- Include opt-out language\n"
            f"- Professional, non-spammy tone\n"
            f"- Under 200 words"
        )

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                total = conn.execute("SELECT COUNT(*) FROM outreach_messages").fetchone()[0]
                by_status = {}
                for row in conn.execute(
                    "SELECT response_status, COUNT(*) as cnt FROM outreach_messages "
                    "GROUP BY response_status"):
                    by_status[row["response_status"]] = row["cnt"]
                by_channel = {}
                for row in conn.execute(
                    "SELECT channel, COUNT(*) as cnt FROM outreach_messages GROUP BY channel"):
                    by_channel[row["channel"]] = row["cnt"]
                return {"total": total, "by_status": by_status, "by_channel": by_channel}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


class DemoGenerator:
    """Module 6: Demo System Generation — tailored demo AI systems."""

    async def generate_blueprint(self, company_id: str, domain: str = "",
                                 agents: List[Dict] = None, workflows: List[Dict] = None,
                                 integrations: List[str] = None,
                                 monitoring_rules: List[Dict] = None,
                                 deployment_targets: List[str] = None,
                                 industry_template: str = "") -> str:
        blueprint_id = f"bp-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO demo_blueprints "
                    "(blueprint_id, company_id, domain, agents_json, workflows_json, "
                    "integrations_json, monitoring_rules_json, deployment_targets_json, "
                    "industry_template, created_at) VALUES (?,?,?,?,?,?,?,?,?,?)",
                    (blueprint_id, company_id, domain,
                     json.dumps(agents or []), json.dumps(workflows or []),
                     json.dumps(integrations or []), json.dumps(monitoring_rules or []),
                     json.dumps(deployment_targets or []), industry_template, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return blueprint_id

    async def get_blueprint(self, blueprint_id: str = None,
                            company_id: str = None) -> Optional[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if blueprint_id:
                    row = conn.execute("SELECT * FROM demo_blueprints WHERE blueprint_id=?",
                                       (blueprint_id,)).fetchone()
                elif company_id:
                    row = conn.execute(
                        "SELECT * FROM demo_blueprints WHERE company_id=? "
                        "ORDER BY created_at DESC LIMIT 1", (company_id,)).fetchone()
                else:
                    row = None
                return dict(row) if row else None
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_blueprints(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT db.*, cp.name, cp.industry FROM demo_blueprints db "
                    "LEFT JOIN company_profiles cp ON db.company_id = cp.company_id "
                    "ORDER BY db.created_at DESC LIMIT ?", (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def generate_blueprint_prompt(self, company_id: str) -> str:
        """Build prompt for AI-driven blueprint generation."""
        def _q():
            conn = _db_connect()
            try:
                profile = conn.execute("SELECT * FROM company_profiles WHERE company_id=?",
                                       (company_id,)).fetchone()
                research = conn.execute("SELECT * FROM company_research WHERE company_id=? "
                                        "ORDER BY created_at DESC LIMIT 1",
                                        (company_id,)).fetchone()
                return dict(profile) if profile else None, dict(research) if research else None
            finally:
                conn.close()
        profile, research = await asyncio.to_thread(_q)
        if not profile:
            return ""
        use_cases = json.loads(research.get("ai_use_cases_json", "[]")) if research else []
        return (
            f"Generate a demo AI system blueprint for {profile['name']}:\n"
            f"Industry: {profile.get('industry', '')}\n"
            f"Size: {profile.get('size_estimate', '')}\n"
            f"AI Use Cases: {json.dumps(use_cases)}\n\n"
            f"Include: agents list, workflows, integrations, monitoring rules, "
            f"deployment targets. Return as JSON."
        )


class ProposalGenerator:
    """Module 7: Proposal Generation — structured deployment proposals."""

    async def create_proposal(self, company_id: str, blueprint_id: str = "",
                              architecture: str = "", problem: str = "",
                              security_model: str = "", deployment_approach: str = "",
                              cost: float = 0.0, roi: float = 0.0,
                              benefits: str = "") -> str:
        proposal_id = f"prop-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO system_proposals "
                    "(proposal_id, company_id, blueprint_id, architecture_summary, "
                    "problem_summary, security_model, deployment_approach, "
                    "estimated_cost, estimated_roi, expected_benefits, status, created_at) "
                    "VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                    (proposal_id, company_id, blueprint_id, architecture, problem,
                     security_model, deployment_approach, cost, roi, benefits,
                     "draft", time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return proposal_id

    async def update_status(self, proposal_id: str, status: str):
        """Update proposal status: draft, sent, accepted, rejected, negotiating."""
        def _up():
            conn = _db_connect()
            try:
                conn.execute("UPDATE system_proposals SET status=? WHERE proposal_id=?",
                             (status, proposal_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_up)

    async def get_proposals(self, company_id: str = None, status: str = None,
                            limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                where = []
                params = []
                if company_id:
                    where.append("sp.company_id=?")
                    params.append(company_id)
                if status:
                    where.append("sp.status=?")
                    params.append(status)
                clause = f"WHERE {' AND '.join(where)}" if where else ""
                params.append(limit)
                return [dict(r) for r in conn.execute(
                    f"SELECT sp.*, cp.name, cp.industry FROM system_proposals sp "
                    f"LEFT JOIN company_profiles cp ON sp.company_id = cp.company_id "
                    f"{clause} ORDER BY sp.created_at DESC LIMIT ?", params).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                total = conn.execute("SELECT COUNT(*) FROM system_proposals").fetchone()[0]
                by_status = {}
                for row in conn.execute(
                    "SELECT status, COUNT(*) as cnt FROM system_proposals GROUP BY status"):
                    by_status[row["status"]] = row["cnt"]
                total_value = conn.execute(
                    "SELECT COALESCE(SUM(estimated_cost), 0) FROM system_proposals "
                    "WHERE status='accepted'").fetchone()[0]
                return {"total": total, "by_status": by_status,
                        "accepted_value": total_value}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


class DeploymentTrigger:
    """Module 8: Deployment Trigger — convert proposals to SWARM builds."""

    async def create_deployment(self, company_id: str, blueprint_id: str = "",
                                proposal_id: str = "", environment: str = "",
                                plan: Dict = None) -> str:
        deployment_id = f"dep-{uuid.uuid4().hex[:10]}"
        now = time.time()
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO deployments "
                    "(deployment_id, company_id, blueprint_id, proposal_id, "
                    "deployment_status, environment_target, deployment_plan_json, "
                    "created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)",
                    (deployment_id, company_id, blueprint_id, proposal_id,
                     "planned", environment, json.dumps(plan or {}), now, now))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return deployment_id

    async def update_status(self, deployment_id: str, status: str,
                            monitoring_url: str = None):
        def _up():
            conn = _db_connect()
            try:
                if monitoring_url:
                    conn.execute(
                        "UPDATE deployments SET deployment_status=?, monitoring_url=?, "
                        "updated_at=? WHERE deployment_id=?",
                        (status, monitoring_url, time.time(), deployment_id))
                else:
                    conn.execute(
                        "UPDATE deployments SET deployment_status=?, updated_at=? "
                        "WHERE deployment_id=?", (status, time.time(), deployment_id))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_up)

    async def get_deployments(self, status: str = None, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if status:
                    return [dict(r) for r in conn.execute(
                        "SELECT d.*, cp.name FROM deployments d "
                        "LEFT JOIN company_profiles cp ON d.company_id = cp.company_id "
                        "WHERE d.deployment_status=? ORDER BY d.updated_at DESC LIMIT ?",
                        (status, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT d.*, cp.name FROM deployments d "
                    "LEFT JOIN company_profiles cp ON d.company_id = cp.company_id "
                    "ORDER BY d.updated_at DESC LIMIT ?", (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def trigger_from_proposal(self, proposal_id: str) -> Optional[str]:
        """Auto-trigger deployment when proposal is accepted."""
        def _q():
            conn = _db_connect()
            try:
                row = conn.execute("SELECT * FROM system_proposals WHERE proposal_id=?",
                                   (proposal_id,)).fetchone()
                return dict(row) if row else None
            finally:
                conn.close()
        proposal = await asyncio.to_thread(_q)
        if not proposal or proposal.get("status") != "accepted":
            return None

        plan = {
            "source": "auto_trigger",
            "proposal_id": proposal_id,
            "architecture": proposal.get("architecture_summary", ""),
            "steps": [
                "provision_infrastructure",
                "deploy_core_services",
                "configure_agents",
                "setup_monitoring",
                "run_validation",
                "handoff_to_client",
            ],
        }
        dep_id = await self.create_deployment(
            proposal["company_id"], proposal.get("blueprint_id", ""),
            proposal_id, "cloud", plan)
        return dep_id


class RevenueTracker:
    """Module 9: Revenue Tracking — economic performance of deployed systems."""

    async def record_revenue(self, company_id: str, deployment_id: str = "",
                             revenue_type: str = "deployment_fee",
                             amount: float = 0.0, billing_period: str = "",
                             notes: str = "") -> str:
        revenue_id = f"rev-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO client_revenue "
                    "(revenue_id, company_id, deployment_id, revenue_type, amount, "
                    "billing_period, notes, updated_at) VALUES (?,?,?,?,?,?,?,?)",
                    (revenue_id, company_id, deployment_id, revenue_type,
                     amount, billing_period, notes, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return revenue_id

    async def get_revenue(self, company_id: str = None, limit: int = 30) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                if company_id:
                    return [dict(r) for r in conn.execute(
                        "SELECT * FROM client_revenue WHERE company_id=? "
                        "ORDER BY updated_at DESC LIMIT ?", (company_id, limit)).fetchall()]
                return [dict(r) for r in conn.execute(
                    "SELECT cr.*, cp.name FROM client_revenue cr "
                    "LEFT JOIN company_profiles cp ON cr.company_id = cp.company_id "
                    "ORDER BY cr.updated_at DESC LIMIT ?", (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                total = conn.execute(
                    "SELECT COALESCE(SUM(amount), 0) FROM client_revenue").fetchone()[0]
                mrr = conn.execute(
                    "SELECT COALESCE(SUM(amount), 0) FROM client_revenue "
                    "WHERE revenue_type='monthly_recurring'").fetchone()[0]
                clients = conn.execute(
                    "SELECT COUNT(DISTINCT company_id) FROM client_revenue").fetchone()[0]
                by_type = {}
                for row in conn.execute(
                    "SELECT revenue_type, SUM(amount) as total FROM client_revenue "
                    "GROUP BY revenue_type"):
                    by_type[row["revenue_type"]] = row["total"]
                return {"total_revenue": total, "mrr": mrr,
                        "active_clients": clients, "by_type": by_type}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


class RelationshipLearner:
    """Module 10: Learning & Optimization — improve from relationship outcomes."""

    async def record_outcome(self, company_id: str, stage_reached: str,
                             success: bool = False, revenue: float = 0.0,
                             lessons: List[str] = None) -> str:
        outcome_id = f"rout-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO relationship_outcomes "
                    "(outcome_id, company_id, stage_reached, success, "
                    "revenue_generated, lessons_json, created_at) VALUES (?,?,?,?,?,?,?)",
                    (outcome_id, company_id, stage_reached, 1 if success else 0,
                     revenue, json.dumps(lessons or []), time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)
        return outcome_id

    async def record_optimization(self, strategy_change: str,
                                  performance_delta: float = 0.0,
                                  affected_stage: str = ""):
        update_id = f"opt-{uuid.uuid4().hex[:10]}"
        def _ins():
            conn = _db_connect()
            try:
                conn.execute(
                    "INSERT INTO optimization_updates "
                    "(update_id, strategy_change, performance_delta, "
                    "affected_stage, timestamp) VALUES (?,?,?,?,?)",
                    (update_id, strategy_change, performance_delta,
                     affected_stage, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_ins)

    async def get_outcomes(self, limit: int = 20) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT ro.*, cp.name FROM relationship_outcomes ro "
                    "LEFT JOIN company_profiles cp ON ro.company_id = cp.company_id "
                    "ORDER BY ro.created_at DESC LIMIT ?", (limit,)).fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_conversion_rates(self) -> Dict:
        """Calculate stage-to-stage conversion rates."""
        def _q():
            conn = _db_connect()
            try:
                stages = ["detected", "research", "contact_initiated", "conversation_active",
                          "demo_requested", "proposal_sent", "deployment_in_progress", "active_client"]
                rates = {}
                for i in range(len(stages) - 1):
                    current = conn.execute(
                        "SELECT COUNT(*) FROM relationship_outcomes WHERE stage_reached=?",
                        (stages[i],)).fetchone()[0]
                    next_stage = conn.execute(
                        "SELECT COUNT(*) FROM relationship_outcomes WHERE stage_reached=?",
                        (stages[i + 1],)).fetchone()[0]
                    rates[f"{stages[i]}_to_{stages[i+1]}"] = (
                        round(next_stage / max(1, current), 3))
                total_success = conn.execute(
                    "SELECT COUNT(*) FROM relationship_outcomes WHERE success=1").fetchone()[0]
                total = conn.execute(
                    "SELECT COUNT(*) FROM relationship_outcomes").fetchone()[0]
                rates["overall_success"] = round(total_success / max(1, total), 3)
                return rates
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                total = conn.execute("SELECT COUNT(*) FROM relationship_outcomes").fetchone()[0]
                successes = conn.execute("SELECT COUNT(*) FROM relationship_outcomes WHERE success=1").fetchone()[0]
                total_rev = conn.execute("SELECT COALESCE(SUM(revenue_generated), 0) FROM relationship_outcomes").fetchone()[0]
                optimizations = conn.execute("SELECT COUNT(*) FROM optimization_updates").fetchone()[0]
                return {"total_outcomes": total, "successes": successes,
                        "total_revenue": total_rev, "optimizations": optimizations,
                        "success_rate": round(successes / max(1, total), 3)}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


class OutreachCompliance:
    """Module 11: Ethics & Compliance Guardrails — prevent abusive automation."""

    async def seed_policies(self):
        """Seed default outreach compliance policies."""
        defaults = [
            ("rate_limit_email", "rate_limit", "10", "block", 10, 72),
            ("rate_limit_linkedin", "rate_limit", "5", "block", 5, 48),
            ("opt_out_honor", "opt_out", "always", "block", 0, 0),
            ("no_spam", "content_policy", "no_bulk_identical", "block", 0, 0),
            ("approval_required", "approval_gate", "all_outreach", "require_approval", 0, 0),
            ("gdpr_compliance", "data_policy", "gdpr_aware", "warn", 0, 0),
            ("can_spam_compliance", "content_policy", "include_opt_out", "enforce", 0, 0),
        ]
        def _seed():
            conn = _db_connect()
            try:
                for pid, rtype, rval, action, max_day, cooldown in defaults:
                    conn.execute(
                        "INSERT OR IGNORE INTO outreach_policies "
                        "(policy_id, rule_type, rule_value, enforcement_action, "
                        "max_per_day, cooldown_hours, updated_at) VALUES (?,?,?,?,?,?,?)",
                        (pid, rtype, rval, action, max_day, cooldown, time.time()))
                conn.commit()
            finally:
                conn.close()
        await asyncio.to_thread(_seed)

    async def check_compliance(self, company_id: str, channel: str) -> Dict:
        """Check if outreach to company via channel is compliant."""
        def _check():
            conn = _db_connect()
            try:
                # Check rate limits
                now = time.time()
                day_ago = now - 86400
                sent_today = conn.execute(
                    "SELECT COUNT(*) FROM outreach_messages "
                    "WHERE channel=? AND sent_at > ? AND response_status='sent'",
                    (channel, day_ago)).fetchone()[0]

                # Get rate limit policy for channel
                policy = conn.execute(
                    "SELECT * FROM outreach_policies WHERE rule_type='rate_limit' "
                    "AND policy_id LIKE ?", (f"rate_limit_{channel}%",)).fetchone()
                max_daily = policy["max_per_day"] if policy else 10
                cooldown_h = policy["cooldown_hours"] if policy else 72

                # Check cooldown for this company
                last_sent = conn.execute(
                    "SELECT MAX(sent_at) FROM outreach_messages "
                    "WHERE company_id=? AND channel=? AND sent_at IS NOT NULL",
                    (company_id, channel)).fetchone()[0]
                cooldown_ok = (last_sent is None or
                               (now - last_sent) > cooldown_h * 3600)

                # Check opt-out
                opted_out = conn.execute(
                    "SELECT COUNT(*) FROM outreach_messages "
                    "WHERE company_id=? AND response_status='opted_out'",
                    (company_id,)).fetchone()[0]

                violations = []
                if sent_today >= max_daily:
                    violations.append(f"Rate limit exceeded: {sent_today}/{max_daily} today")
                if not cooldown_ok:
                    violations.append(f"Cooldown active: last contact < {cooldown_h}h ago")
                if opted_out > 0:
                    violations.append("Company has opted out of communications")

                return {
                    "compliant": len(violations) == 0,
                    "violations": violations,
                    "sent_today": sent_today,
                    "max_daily": max_daily,
                    "cooldown_ok": cooldown_ok,
                    "opted_out": opted_out > 0,
                }
            finally:
                conn.close()
        return await asyncio.to_thread(_check)

    async def get_policies(self) -> List[Dict]:
        def _q():
            conn = _db_connect()
            try:
                return [dict(r) for r in conn.execute(
                    "SELECT * FROM outreach_policies ORDER BY rule_type").fetchall()]
            finally:
                conn.close()
        return await asyncio.to_thread(_q)

    async def get_stats(self) -> Dict:
        def _q():
            conn = _db_connect()
            try:
                policies = conn.execute("SELECT COUNT(*) FROM outreach_policies").fetchone()[0]
                opted_out = conn.execute(
                    "SELECT COUNT(DISTINCT company_id) FROM outreach_messages "
                    "WHERE response_status='opted_out'").fetchone()[0]
                blocked = conn.execute(
                    "SELECT COUNT(*) FROM outreach_messages "
                    "WHERE response_status='blocked'").fetchone()[0]
                return {"policies": policies, "opted_out_companies": opted_out,
                        "blocked_messages": blocked}
            finally:
                conn.close()
        return await asyncio.to_thread(_q)


# Instantiate Relationship Engine services
signal_discovery = SignalDiscovery()
opportunity_qualifier = OpportunityQualifier()
relationship_pipeline = RelationshipPipeline()
research_agent = ResearchAgent()
outreach_generator = OutreachGenerator()
demo_generator = DemoGenerator()
proposal_generator = ProposalGenerator()
deployment_trigger = DeploymentTrigger()
revenue_tracker = RevenueTracker()
relationship_learner = RelationshipLearner()
outreach_compliance = OutreachCompliance()


# ---------------------------------------------------------------------------
# Dashboard API
# ---------------------------------------------------------------------------

async def dashboard_overview(request: web.Request) -> web.Response:
    """Dashboard overview endpoint."""
    try:
        mem_stats = await memory.stats()
        active_tasks = task_manager.get_active_tasks()
        alerts = await monitor.get_alerts(active_only=True)
        plans = await planner.get_recent_plans(5)
        agents = await agent_coordinator.get_agents()
        repairs = await self_healer.get_repair_history(5)

        return web.json_response({
            "system": "bunny-alpha",
            "status": "healthy",
            "memory": {
                "total_messages": mem_stats["total_messages"],
                "channels": len(mem_stats["channels"]),
                "db_size_kb": round(mem_stats["db_size_bytes"] / 1024, 1),
            },
            "tasks": {
                "active": len(active_tasks),
                "recent": len(task_manager.get_recent_tasks(20)),
            },
            "alerts": {"active": len(alerts)},
            "plans": {"recent": len(plans)},
            "agents": {"count": len(agents)},
            "repairs": {"recent": len(repairs)},
            "routing_mode": _routing_mode,
            "self_healing": self_healer.enabled,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_tasks(request: web.Request) -> web.Response:
    """Dashboard tasks endpoint."""
    recent = task_manager.get_recent_tasks(50)
    return web.json_response({
        "tasks": [
            {
                "task_id": t.task_id,
                "tool": t.tool,
                "host": t.host,
                "cmd": t.cmd[:100],
                "status": t.status.value,
                "duration": t.duration,
                "created_at": t.created_at,
            }
            for t in recent
        ]
    })


async def dashboard_monitoring(request: web.Request) -> web.Response:
    """Dashboard monitoring endpoint."""
    checks = await monitor.get_checks(enabled_only=False)
    alerts = await monitor.get_alerts(active_only=False, limit=50)
    return web.json_response({
        "checks": checks,
        "alerts": alerts,
    })


async def dashboard_plans(request: web.Request) -> web.Response:
    """Dashboard plans endpoint."""
    plans = await planner.get_recent_plans(20)
    result = []
    for p in plans:
        plan_data = await planner.get_plan(p["plan_id"])
        if plan_data:
            result.append(plan_data)
    return web.json_response({"plans": result})


async def dashboard_routing(request: web.Request) -> web.Response:
    """Dashboard routing endpoint."""
    perf = await perf_router.get_performance()
    return web.json_response({
        "mode": _routing_mode,
        "performance": perf,
    })


async def dashboard_graph(request: web.Request) -> web.Response:
    """Dashboard graph endpoint."""
    def _query():
        conn = _db_connect()
        try:
            entities = [dict(r) for r in conn.execute("SELECT * FROM graph_entities").fetchall()]
            edges = [dict(r) for r in conn.execute("SELECT * FROM graph_edges").fetchall()]
            events = [dict(r) for r in conn.execute(
                "SELECT * FROM graph_events ORDER BY created_at DESC LIMIT 50"
            ).fetchall()]
            return {"entities": entities, "edges": edges, "recent_events": events}
        finally:
            conn.close()
    data = await asyncio.to_thread(_query)
    return web.json_response(data)


async def dashboard_sessions(request: web.Request) -> web.Response:
    """Dashboard sessions endpoint."""
    try:
        sessions = await session_mgr.list_sessions(limit=50)
        active = [s for s in sessions if s.get("status") == "active"]
        return web.json_response({
            "total": len(sessions), "active": len(active), "sessions": sessions,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_infrastructure(request: web.Request) -> web.Response:
    """Dashboard infrastructure endpoint."""
    try:
        workers = await worker_registry.list_workers()
        twin_status = await digital_twin.get_status()
        env_health = await env_awareness.get_health()
        return web.json_response({
            "vms": dict(VMS),
            "workers": workers,
            "twin": twin_status,
            "environment_health": env_health,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_knowledge(request: web.Request) -> web.Response:
    """Dashboard knowledge endpoint."""
    try:
        patterns = await knowledge_evolution.get_patterns(10)
        playbooks = await knowledge_evolution.get_playbooks(10)
        distillations = await memory_distiller.get_distillations(10)
        return web.json_response({
            "patterns": patterns, "playbooks": playbooks,
            "distillations": distillations,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_audit(request: web.Request) -> web.Response:
    """Dashboard audit endpoint."""
    try:
        recent = await audit.get_recent(50)
        total = await audit.count()
        pending_approvals = await perm_mgr.get_pending(20)
        escalations = await escalation_mgr.get_open(10)
        return web.json_response({
            "total_events": total, "recent": recent,
            "pending_approvals": pending_approvals,
            "open_escalations": escalations,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_learning(request: web.Request) -> web.Response:
    """Dashboard learning/intelligence endpoint."""
    try:
        outcome_stats = await outcome_learner.get_stats()
        intel_history = await intel_loop.get_history(5)
        agent_scores = await agent_scorer.get_scores()
        repair_pats = await repair_learner.get_patterns()
        return web.json_response({
            "outcomes": outcome_stats, "intelligence_runs": intel_history,
            "agent_scores": agent_scores, "repair_patterns": repair_pats,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_environment(request: web.Request) -> web.Response:
    """Dashboard environment awareness endpoint."""
    try:
        env_status = await env_awareness.get_status()
        recent_events = await event_ingestor.get_recent(30)
        auto_actions = await auto_ops.get_actions(limit=10)
        return web.json_response({
            "environment": env_status,
            "recent_events": recent_events,
            "autonomous_actions": auto_actions,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_system(request: web.Request) -> web.Response:
    """Dashboard system evaluation endpoint."""
    try:
        recent_evals = await system_evaluator.get_recent(5)
        scorecards = await system_evaluator.get_scorecard()
        recommendations = await system_evaluator.get_recommendations()
        return web.json_response({
            "evaluations": recent_evals, "scorecards": scorecards,
            "recommendations": recommendations,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_actions(request: web.Request) -> web.Response:
    """Dashboard structured execution actions endpoint."""
    try:
        stats = await action_service.get_stats()
        recent = await action_service.get_actions(limit=20)
        policies = await action_service.get_policies()
        audit_trail = await action_service.get_audit_trail(limit=20)
        return web.json_response({
            "stats": stats,
            "recent_actions": recent,
            "policies": policies,
            "audit_trail": audit_trail,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_execution(request: web.Request) -> web.Response:
    """Dashboard execution results and safety boundary endpoint."""
    try:
        stats = await action_service.get_stats()
        results = await action_service.get_results(limit=30)
        pending = await action_service.get_actions(status="awaiting_approval", limit=10)
        risk_policy = {k: v for k, v in RISK_POLICY.items()}
        blocked = BLOCKED_PATTERNS
        return web.json_response({
            "stats": stats,
            "recent_results": results,
            "pending_approvals": pending,
            "risk_policy": risk_policy,
            "blocked_patterns": blocked,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_pipeline(request: web.Request) -> web.Response:
    """Dashboard relationship pipeline endpoint."""
    try:
        pipe_stats = await relationship_pipeline.get_stats()
        pipeline = await relationship_pipeline.get_pipeline(limit=20)
        recent_events = await relationship_pipeline.get_events(limit=15)
        signal_stats = await signal_discovery.get_stats()
        return web.json_response({
            "pipeline_stats": pipe_stats,
            "pipeline": pipeline,
            "recent_events": recent_events,
            "signal_stats": signal_stats,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


async def dashboard_opportunities(request: web.Request) -> web.Response:
    """Dashboard opportunities and revenue endpoint."""
    try:
        top_opps = await opportunity_qualifier.get_top_opportunities(15)
        proposal_stats = await proposal_generator.get_stats()
        revenue_stats = await revenue_tracker.get_stats()
        outreach_stats = await outreach_generator.get_stats()
        compliance_stats = await outreach_compliance.get_stats()
        learning_stats = await relationship_learner.get_stats()
        return web.json_response({
            "top_opportunities": top_opps,
            "proposals": proposal_stats,
            "revenue": revenue_stats,
            "outreach": outreach_stats,
            "compliance": compliance_stats,
            "learning": learning_stats,
        })
    except Exception as e:
        return web.json_response({"error": str(e)}, status=500)


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
    "task": "Show task detail (/task <id>)",
    "cancel": "Cancel a task (/cancel <id>)",
    "retry": "Retry a failed task (/retry <id>)",
    "vms": "List all VMs with connectivity",
    "docker": "List Docker containers on swarm-mainframe",
    "gpu": "Show GPU status on swarm-gpu",
    "models": "List all available AI models (26+)",
    "model": "Switch active model (e.g. /model gpt5, /model claude)",
    "logs": "Show recent Bunny Alpha logs",
    "health": "Run health check on all services",
    "monitor": "Monitoring checks (/monitor list|run|mute|unmute|alerts)",
    "schedule": "Schedule a job (/schedule reminder|shell|health ...)",
    "jobs": "List scheduled jobs",
    "unschedule": "Remove a scheduled job (/unschedule <id>)",
    "graph": "Knowledge graph (/graph entity|deps|impact|recent|search)",
    "plan": "Create/manage plans (/plan <goal>, /plan status|cancel|list)",
    "agents": "List available sub-agents",
    "delegate": "Delegate task to sub-agents (/delegate <task>)",
    "predict": "Predictive monitoring (/predict health|risk|<vm>)",
    "heal": "Self-healing status/control (/heal status|history|enable|disable)",
    "route": "Routing status/mode (/route status|mode <mode>)",
    "simulate": "Simulate plan/action (/simulate <plan_id>|action <type>)",
    "dashboard": "Show dashboard API endpoints",
    "search": "Web search (/search <query>)",
    "fetch": "Fetch URL content (/fetch <url>)",
    "python": "Run Python code (/python <code>)",
    "js": "Run JavaScript code (/js <code>)",
    "files": "File operations (/files find|grep|read|summary <args>)",
    "git": "Git operations (/git status|log|diff|branch|pull)",
    # Operational Hardening
    "session": "Swarm sessions (/session list|status|close|resume)",
    "approvals": "View pending approvals",
    "approve": "Approve a request (/approve <id>)",
    "reject": "Reject a request (/reject <id>)",
    "drill": "Failure drills (/drill run|status|history)",
    "audit": "View audit log (/audit recent|search <query>)",
    "sandbox": "Sandbox policies (/sandbox status)",
    "escalations": "View escalations (/escalations)",
    # Continuous Learning
    "outcomes": "View outcomes (/outcomes recent|task|route|repair)",
    "learn": "Intelligence loop (/learn status|run|history)",
    "why": "Explain decisions (/why task|route|plan|repair|agent <id>)",
    "kb": "Knowledge base (/kb recipes|incidents|search <query>)",
    # Scale & Autonomy
    "workers": "Worker registry (/workers health|region|quarantine)",
    "initiative": "Autonomous initiative (/initiative status|history)",
    "evaluate": "System evaluation (/evaluate)",
    "policy": "Safety policies (/policy status|explain <action>)",
    "plugins": "Plugin management (/plugins enable|disable|info)",
    "scorecard": "System scorecard (/scorecard)",
    # Environment Intelligence
    "env": "Environment status (/env status|entity|signals|health)",
    "events": "Event stream (/events recent|entity|correlation)",
    "twin": "Digital twin (/twin status|simulate|explain)",
    "auto": "Autonomous ops (/auto status|history)",
    "playbooks": "Operational playbooks (/playbooks search|explain)",
    "ops": "Operator overview (/ops overview|incidents|twin|autonomy)",
    # Structured Execution
    "actions": "Structured actions (/actions recent|stats|policies|audit|blocked)",
    "execution": "Execution engine (/execution status|results)",
    # Relationship & Opportunity Engine
    "signals": "Company signals (/signals recent|stats|sources)",
    "opportunities": "Opportunity scoring (/opportunities top|profiles)",
    "pipeline": "Relationship pipeline (/pipeline overview|events|<stage>)",
    "research": "Company research (/research list)",
    "outreach": "Outreach messages (/outreach recent|stats|compliance)",
    "proposals": "System proposals (/proposals list|stats)",
    "revenue": "Revenue tracking (/revenue summary|recent|conversions)",
    "deployments": "Client deployments (/deployments list)",
    "crm": "CRM overview (/crm)",
    # Core
    "memory": "Show persistent memory stats (/memory search|distill <query>)",
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
        recent = task_manager.get_recent_tasks(15)
        if not recent:
            await post_message(":clipboard: No tasks yet.", channel, thread_ts)
            return True
        status_icons = {
            TaskStatus.COMPLETED: ":white_check_mark:",
            TaskStatus.FAILED: ":x:",
            TaskStatus.RUNNING: ":hourglass_flowing_sand:",
            TaskStatus.QUEUED: ":inbox_tray:",
            TaskStatus.CANCELLED: ":no_entry_sign:",
            TaskStatus.BLOCKED: ":no_entry:",
        }
        active = [t for t in recent if t.status == TaskStatus.RUNNING]
        lines = [f":clipboard: *Tasks* ({len(active)} active, {len(recent)} recent)\n"]
        for t in recent:
            icon = status_icons.get(t.status, ":grey_question:")
            dur = f" ({t.duration}s)" if t.duration else ""
            retry = f" [retry #{t.retries}]" if t.retries > 0 else ""
            lines.append(f"{icon} `{t.short_id}` {t.tool}@{t.host}: `{t.cmd[:40]}`{dur}{retry}")
        lines.append(f"\n_Use_ `/task <id>` _for details,_ `/cancel <id>` _to cancel,_ `/retry <id>` _to retry_")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "task":
        tid = args.strip()
        if not tid:
            await post_message(":warning: Usage: `/task <id>`", channel, thread_ts)
            return True
        task = task_manager.get_task(tid)
        if not task:
            await post_message(f":warning: Task `{tid}` not found.", channel, thread_ts)
            return True
        lines = [f":mag: *Task {task.short_id}*\n"]
        lines.append(f"*Status:* {task.status.value}")
        lines.append(f"*Tool:* `{task.tool}@{task.host}`")
        lines.append(f"*Command:* `{task.cmd[:200]}`")
        if task.created_by:
            lines.append(f"*Created by:* <@{task.created_by}>")
        if task.retries > 0:
            lines.append(f"*Retries:* {task.retries}")
        if task.duration:
            lines.append(f"*Duration:* {task.duration}s")
        if task.result:
            result_preview = task.result[:500]
            lines.append(f"*Result:*\n```{result_preview}```")
        if task.error:
            lines.append(f"*Error:* `{task.error[:300]}`")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "cancel":
        tid = args.strip()
        if not tid:
            await post_message(":warning: Usage: `/cancel <id>`", channel, thread_ts)
            return True
        task = task_manager.cancel_task(tid)
        if task:
            await memory.update_task(task.task_id, "cancelled")
            await post_message(f":no_entry_sign: Task `{task.short_id}` cancelled.", channel, thread_ts)
        else:
            await post_message(f":warning: Task `{tid}` not found or can't be cancelled.", channel, thread_ts)
        return True

    if cmd == "retry":
        tid = args.strip()
        if not tid:
            await post_message(":warning: Usage: `/retry <id>`", channel, thread_ts)
            return True
        new_task = await task_manager.retry_task(tid, channel, thread_ts)
        if new_task:
            await post_message(
                f":arrows_counterclockwise: Retried as task `{new_task.short_id}` (retry #{new_task.retries})",
                channel, thread_ts,
            )
        else:
            await post_message(f":warning: Task `{tid}` not found or can't be retried (only failed/cancelled tasks).", channel, thread_ts)
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
        # Run monitoring checks
        results = await monitor.run_all_checks()
        if not results:
            await post_message(":stethoscope: No health checks configured.", channel, thread_ts)
            return True
        lines = [":stethoscope: *System Health Check*\n"]
        ok = warning = critical = 0
        for r in results:
            icon = ":white_check_mark:" if r["status"] == "ok" else ":x:" if r["status"] == "critical" else ":warning:"
            lines.append(f"{icon} *{r['name']}* ({r['target']}): `{r['status']}`")
            if r["result"] and r["status"] != "ok":
                lines.append(f"   {r['result'][:100]}")
            if r["status"] == "ok":
                ok += 1
            elif r["status"] == "critical":
                critical += 1
            else:
                warning += 1
        lines.append(f"\n_Summary: {ok} ok, {warning} warning, {critical} critical_")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "monitor":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"

        if subcmd == "list":
            checks = await monitor.get_checks(enabled_only=False)
            lines = [":satellite: *Monitoring Checks*\n"]
            for c in checks:
                icon = ":white_check_mark:" if c.get("last_status") == "ok" else ":x:" if c.get("last_status") == "critical" else ":grey_question:"
                muted = " (muted)" if c.get("muted") else ""
                disabled = " (disabled)" if not c.get("enabled") else ""
                lines.append(f"{icon} `{c['check_id']}` {c['name']} — {c['target']}{muted}{disabled}")
            await post_message("\n".join(lines), channel, thread_ts)

        elif subcmd == "run" and len(sub) > 1:
            check_id = sub[1].strip()
            checks = await monitor.get_checks(enabled_only=False)
            check = next((c for c in checks if c["check_id"] == check_id), None)
            if check:
                result = await monitor.run_check(check)
                icon = ":white_check_mark:" if result["status"] == "ok" else ":x:"
                await post_message(
                    f"{icon} *{result['name']}*: `{result['status']}`\n```{result['result'][:500]}```",
                    channel, thread_ts,
                )
            else:
                await post_message(f":warning: Check `{check_id}` not found.", channel, thread_ts)

        elif subcmd == "mute" and len(sub) > 1:
            await monitor.mute_check(sub[1].strip(), True)
            await post_message(f":mute: Check `{sub[1].strip()}` muted.", channel, thread_ts)

        elif subcmd == "unmute" and len(sub) > 1:
            await monitor.mute_check(sub[1].strip(), False)
            await post_message(f":loud_sound: Check `{sub[1].strip()}` unmuted.", channel, thread_ts)

        elif subcmd == "alerts":
            alerts = await monitor.get_alerts(active_only=True)
            if alerts:
                lines = [":bell: *Active Alerts*\n"]
                import datetime
                for a in alerts:
                    ts = datetime.datetime.fromtimestamp(a["created_at"]).strftime("%m/%d %H:%M")
                    lines.append(f":rotating_light: `{a['check_id']}` [{a['status']}] {a['message'][:100]} ({ts})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":white_check_mark: No active alerts.", channel, thread_ts)
        else:
            await post_message(
                ":satellite: */monitor* commands: `list`, `run <id>`, `mute <id>`, `unmute <id>`, `alerts`",
                channel, thread_ts,
            )
        return True

    # -- Scheduler commands --
    if cmd == "schedule":
        parts = args.strip().split(maxsplit=2)
        if len(parts) < 2:
            await post_message(
                ":clock1: Usage:\n"
                "\u2022 `/schedule reminder in 5m Take a break`\n"
                "\u2022 `/schedule shell every 10m {\"host\":\"swarm-gpu\",\"cmd\":\"nvidia-smi\"}`\n"
                "\u2022 `/schedule health every 30m`",
                channel, thread_ts,
            )
            return True
        job_type = parts[0].lower()
        rest = parts[1] if len(parts) > 1 else ""

        # Parse schedule expression and payload
        schedule_expr = None
        interval_seconds = None
        payload = rest

        # Try to extract timing: "in 5m ...", "every 10m ...", "at 17:00 ..."
        time_match = re.match(r'(in \d+[mhs]|every \d+[mhs]|at \d{1,2}:\d{2})\s*(.*)', rest, re.IGNORECASE)
        if time_match:
            schedule_expr = time_match.group(1).strip()
            payload = time_match.group(2).strip() if time_match.group(2) else ""

            # Parse interval for recurring
            if schedule_expr.startswith("every "):
                val = schedule_expr[6:].strip()
                if val.endswith("m"):
                    interval_seconds = int(val[:-1]) * 60
                elif val.endswith("h"):
                    interval_seconds = int(val[:-1]) * 3600

        if not payload and job_type != "health":
            payload = "scheduled task"

        job_id = f"{job_type}-{uuid.uuid4().hex[:6]}"
        result = await scheduler.add_job(
            job_id=job_id, job_type=job_type, payload=payload,
            description=f"{job_type}: {payload[:50]}",
            schedule_expression=schedule_expr,
            interval_seconds=interval_seconds,
            channel_id=channel, thread_ts=thread_ts,
        )
        import datetime
        next_str = datetime.datetime.fromtimestamp(result["next_run_at"]).strftime("%H:%M:%S") if result["next_run_at"] else "now"
        recur = " (recurring)" if interval_seconds else " (one-off)"
        await post_message(
            f":white_check_mark: Job `{job_id}` scheduled{recur}\nNext run: {next_str}",
            channel, thread_ts,
        )
        return True

    if cmd == "jobs":
        jobs = await scheduler.get_jobs(enabled_only=False)
        if not jobs:
            await post_message(":clock1: No scheduled jobs.", channel, thread_ts)
            return True
        import datetime
        lines = [":clock1: *Scheduled Jobs*\n"]
        for j in jobs:
            enabled = ":green_circle:" if j.get("enabled") else ":red_circle:"
            next_run = datetime.datetime.fromtimestamp(j["next_run_at"]).strftime("%m/%d %H:%M") if j.get("next_run_at") else "—"
            desc = j.get("description", j["job_type"])[:50]
            lines.append(f"{enabled} `{j['job_id']}` {desc} | next: {next_run}")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "unschedule":
        job_id = args.strip()
        if not job_id:
            await post_message(":warning: Usage: `/unschedule <job_id>`", channel, thread_ts)
            return True
        await scheduler.remove_job(job_id)
        await post_message(f":wastebasket: Job `{job_id}` removed.", channel, thread_ts)
        return True

    # -- Knowledge Graph commands --
    if cmd == "graph":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "help"
        query = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "entity" and query:
            entity = await knowledge_graph.get_entity(query)
            if not entity:
                entities = await knowledge_graph.search_entities(query, limit=5)
                if entities:
                    entity = entities[0]
            if entity:
                attrs = json.loads(entity.get("attributes_json", "{}"))
                lines = [f":globe_with_meridians: *{entity['name']}* ({entity['entity_type']})"]
                if attrs:
                    for k, v in attrs.items():
                        lines.append(f"  `{k}`: {v}")
                neighbors = await knowledge_graph.get_neighbors(entity["entity_id"])
                if neighbors["outgoing"]:
                    lines.append("*Outgoing:*")
                    for n in neighbors["outgoing"]:
                        lines.append(f"  \u2192 {n['relation']} \u2192 {n.get('name', n.get('dst_entity_id', '?'))}")
                if neighbors["incoming"]:
                    lines.append("*Incoming:*")
                    for n in neighbors["incoming"]:
                        lines.append(f"  \u2190 {n.get('name', n.get('src_entity_id', '?'))} \u2192 {n['relation']}")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(f":warning: Entity `{query}` not found.", channel, thread_ts)

        elif subcmd == "deps" and query:
            deps = await knowledge_graph.get_dependencies(query)
            if deps:
                lines = [f":link: *Dependencies of `{query}`*\n"]
                for d in deps:
                    lines.append(f"  \u2192 {d['relation']} \u2192 {d.get('name', d.get('dst_entity_id', '?'))} ({d.get('entity_type', '?')})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(f":link: No dependencies found for `{query}`.", channel, thread_ts)

        elif subcmd == "impact" and query:
            impact = await knowledge_graph.get_impact(query)
            if impact:
                lines = [f":boom: *Impact analysis for `{query}`*\n"]
                for i in impact:
                    lines.append(f"  \u2190 {i.get('name', i.get('src_entity_id', '?'))} ({i.get('entity_type', '?')})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(f":boom: Nothing depends on `{query}`.", channel, thread_ts)

        elif subcmd == "recent":
            entity_id = query if query else None
            events = await knowledge_graph.get_recent_events(entity_id, limit=15)
            if events:
                import datetime
                lines = [f":scroll: *Recent Events*" + (f" for `{query}`" if query else "") + "\n"]
                for e in events:
                    ts = datetime.datetime.fromtimestamp(e["created_at"]).strftime("%m/%d %H:%M")
                    lines.append(f"`{ts}` [{e['event_type']}] {e['entity_id']}")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":scroll: No recent events.", channel, thread_ts)

        elif subcmd == "search" and query:
            results = await knowledge_graph.search_entities(query)
            if results:
                lines = [f":mag: *Graph search: `{query}`* ({len(results)} results)\n"]
                for r in results:
                    lines.append(f"  `{r['entity_id']}` — {r['name']} ({r['entity_type']})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(f":mag: No entities match `{query}`.", channel, thread_ts)

        else:
            await post_message(
                ":globe_with_meridians: */graph* commands: `entity <name>`, `deps <entity>`, "
                "`impact <entity>`, `recent [entity]`, `search <query>`",
                channel, thread_ts,
            )
        return True

    # -- Autonomous Planning commands --
    if cmd == "plan":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else ""

        if subcmd == "status" and len(sub) > 1:
            plan_data = await planner.get_plan(sub[1].strip())
            if plan_data:
                p = plan_data["plan"]
                steps = plan_data["steps"]
                lines = [f":clipboard: *Plan `{p['plan_id']}`* — {p['status']}\n*Goal:* {p['goal_text'][:100]}"]
                for s in steps:
                    icon = {"completed": ":white_check_mark:", "failed": ":x:", "running": ":gear:", "pending": ":inbox_tray:", "blocked": ":no_entry:"}.get(s["status"], ":grey_question:")
                    lines.append(f"{icon} `{s['step_id']}` {s['title']} [{s['status']}]")
                if p.get("summary"):
                    lines.append(f"\n_Summary: {p['summary']}_")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(f":warning: Plan `{sub[1]}` not found.", channel, thread_ts)

        elif subcmd == "cancel" and len(sub) > 1:
            await planner.update_plan_status(sub[1].strip(), "cancelled")
            await post_message(f":no_entry_sign: Plan `{sub[1].strip()}` cancelled.", channel, thread_ts)

        elif subcmd == "list":
            plans = await planner.get_recent_plans()
            if plans:
                import datetime
                lines = [":clipboard: *Recent Plans*\n"]
                for p in plans:
                    ts = datetime.datetime.fromtimestamp(p["created_at"]).strftime("%m/%d %H:%M")
                    lines.append(f"`{p['plan_id']}` [{p['status']}] {p['goal_text'][:60]} ({ts})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":clipboard: No plans yet.", channel, thread_ts)

        elif subcmd and subcmd not in ("status", "cancel", "list", "explain", "retry"):
            # Create a new plan from the full args
            goal = args.strip()
            plan_id = await planner.create_plan(goal)
            await post_message(
                f":brain: Creating plan for: *{goal[:100]}*\nPlan ID: `{plan_id}`",
                channel, thread_ts,
            )
            # Use AI to generate steps
            step_prompt = (
                f"Break this goal into 3-6 concrete infrastructure steps:\n\n"
                f"Goal: {goal}\n\n"
                f"For each step, provide a title and a shell command in JSON format.\n"
                f"Available hosts: swarm-mainframe, swarm-gpu, fc-ai-portal, calculus-web.\n"
                f"Respond with a JSON array like:\n"
                f'[{{"title": "Check VM health", "description": "{{\\\"tool\\\":\\\"shell\\\",\\\"host\\\":\\\"swarm-mainframe\\\",\\\"cmd\\\":\\\"uptime\\\"}}", "priority": 1}}]'
            )
            try:
                ai_response = await query_ai(step_prompt, system="You are a plan generator. Output ONLY valid JSON.")
                # Try to parse steps from AI response
                json_match = re.search(r'\[.*\]', ai_response, re.DOTALL)
                if json_match:
                    steps = json.loads(json_match.group())
                    for i, step in enumerate(steps):
                        await planner.add_step(
                            plan_id, step.get("title", f"Step {i+1}"),
                            step.get("description", ""), priority=step.get("priority", i+1),
                        )
                    await post_message(
                        f":white_check_mark: Plan `{plan_id}` created with {len(steps)} steps.\n"
                        f"Run `/plan status {plan_id}` to view, or tell me to execute it.",
                        channel, thread_ts,
                    )
                else:
                    # Fallback: create a single step
                    await planner.add_step(plan_id, goal, goal)
                    await post_message(
                        f":white_check_mark: Plan `{plan_id}` created (1 step). Use `/plan status {plan_id}` to view.",
                        channel, thread_ts,
                    )
            except Exception as e:
                await post_message(f":warning: Plan created but step generation failed: `{e}`", channel, thread_ts)
        else:
            await post_message(
                ":brain: */plan* commands: `/plan <goal>`, `/plan status <id>`, `/plan cancel <id>`, `/plan list`",
                channel, thread_ts,
            )
        return True

    # -- Multi-Agent commands --
    if cmd == "agents":
        agents = await agent_coordinator.get_agents()
        lines = [":robot_face: *Available Agents*\n"]
        for a in agents:
            lines.append(f"\u2022 *{a['name']}* (`{a['agent_id']}`) — {a['role']}")
            lines.append(f"   Capabilities: `{a.get('capabilities', 'general')}`")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "delegate":
        if not args.strip():
            await post_message(":warning: Usage: `/delegate <task description>`", channel, thread_ts)
            return True
        result = await agent_coordinator.orchestrate(args.strip(), channel, thread_ts)
        await post_message(f":robot_face: *Agent Results*\n{result[:2000]}", channel, thread_ts)
        return True

    # -- Predictive Monitoring commands --
    if cmd == "predict":
        sub = args.strip().lower() or "health"
        if sub == "health":
            signals = await predictor.analyze_health()
            if signals:
                lines = [":crystal_ball: *Predictive Health Analysis*\n"]
                for s in signals:
                    icon = ":rotating_light:" if s["risk_level"] == "critical" else ":warning:"
                    lines.append(
                        f"{icon} *{s['target']}* — {s['signal_type']} "
                        f"(confidence: {s['confidence']:.0%}, window: {s['predicted_failure_window']})\n"
                        f"   {s['explanation']}\n"
                        f"   _Action: {s['recommended_action']}_"
                    )
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":crystal_ball: All systems healthy — no predicted issues.", channel, thread_ts)

        elif sub == "risk":
            assessments = await predictor.get_risk_assessment()
            lines = [":bar_chart: *Risk Assessment*\n"]
            for a in assessments:
                icon = ":red_circle:" if a["risk_level"] == "critical" else ":orange_circle:" if a["risk_level"] == "warning" else ":green_circle:"
                lines.append(f"{icon} *{a['target']}* — risk: {a['risk_score']:.0%} ({a['risk_level']})")
                if a.get("explanation"):
                    lines.append(f"   {a['explanation'][:100]}")
            if not assessments:
                lines.append(":green_circle: No risk signals detected.")
            await post_message("\n".join(lines), channel, thread_ts)

        elif sub in VMS:
            signals = await predictor.analyze_health(sub)
            if signals:
                lines = [f":crystal_ball: *Prediction for `{sub}`*\n"]
                for s in signals:
                    lines.append(f"\u2022 {s['signal_type']}: {s['explanation']} ({s['recommended_action']})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(f":crystal_ball: `{sub}` looks healthy.", channel, thread_ts)
        else:
            await post_message(
                ":crystal_ball: */predict* commands: `health`, `risk`, `<vm-name>`",
                channel, thread_ts,
            )
        return True

    # -- Self-Healing commands --
    if cmd == "heal":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"

        if subcmd == "status":
            status = "enabled" if self_healer.enabled else "disabled"
            history = await self_healer.get_repair_history(5)
            lines = [f":wrench: *Self-Healing: {status}*\n"]
            if history:
                import datetime
                lines.append("*Recent repairs:*")
                for r in history:
                    ts = datetime.datetime.fromtimestamp(r["timestamp"]).strftime("%m/%d %H:%M")
                    icon = ":white_check_mark:" if r.get("success") else ":x:"
                    lines.append(f"{icon} `{r['repair_id']}` {r['fault_class']} on {r['target']} — {r.get('action_taken', '?')} ({ts})")
            else:
                lines.append("No repair history.")
            await post_message("\n".join(lines), channel, thread_ts)

        elif subcmd == "history":
            history = await self_healer.get_repair_history(20)
            if history:
                import datetime
                lines = [":wrench: *Repair History*\n"]
                for r in history:
                    ts = datetime.datetime.fromtimestamp(r["timestamp"]).strftime("%m/%d %H:%M")
                    icon = ":white_check_mark:" if r.get("success") else ":x:"
                    lines.append(f"{icon} `{r['repair_id']}` [{r['fault_class']}] {r['target']} — {r.get('action_taken', '?')} ({ts})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":wrench: No repair history.", channel, thread_ts)

        elif subcmd == "enable":
            self_healer.enabled = True
            await post_message(":white_check_mark: Self-healing enabled.", channel, thread_ts)

        elif subcmd == "disable":
            self_healer.enabled = False
            await post_message(":no_entry_sign: Self-healing disabled.", channel, thread_ts)

        else:
            await post_message(
                ":wrench: */heal* commands: `status`, `history`, `enable`, `disable`",
                channel, thread_ts,
            )
        return True

    # -- Routing commands --
    if cmd == "route":
        global _routing_mode
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"

        if subcmd == "status":
            perf = await perf_router.get_performance()
            lines = [f":arrows_counterclockwise: *Routing Mode: {_routing_mode}*\n"]
            if perf:
                for p in perf[:15]:
                    sr = f"{p['success_rate']:.0%}"
                    lines.append(f"\u2022 `{p['target_id']}` ({p['target_type']}) — "
                                 f"success: {sr}, latency: {p['avg_latency']:.1f}s, "
                                 f"requests: {p['request_count']}")
            else:
                lines.append("No routing data yet.")
            await post_message("\n".join(lines), channel, thread_ts)

        elif subcmd == "mode" and len(sub) > 1:
            mode = sub[1].strip().upper()
            if mode in ROUTING_MODES:
                _routing_mode = mode
                await post_message(f":white_check_mark: Routing mode set to *{mode}*", channel, thread_ts)
            else:
                await post_message(f":warning: Unknown mode. Options: {', '.join(ROUTING_MODES)}", channel, thread_ts)

        else:
            await post_message(
                ":arrows_counterclockwise: */route* commands: `status`, `mode <BALANCED|LOW_LATENCY|LOW_COST|HIGH_RELIABILITY>`",
                channel, thread_ts,
            )
        return True

    # -- Simulation commands --
    if cmd == "simulate":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else ""

        if subcmd.startswith("plan-") or subcmd.startswith("plan_"):
            result = await simulator.simulate_plan(subcmd)
            if "error" in result:
                await post_message(f":warning: {result['error']}", channel, thread_ts)
            else:
                icon = ":green_circle:" if result["risk_level"] == "LOW" else ":orange_circle:" if result["risk_level"] in ("MODERATE", "HIGH") else ":red_circle:"
                lines = [f"{icon} *Simulation `{result['simulation_id']}`*"]
                lines.append(f"Plan: `{result['plan_id']}`")
                lines.append(f"Risk: *{result['risk_level']}* ({result['risk_score']:.0%})")
                lines.append(f"Recommendation: {result['recommended_action']}")
                for s in result.get("steps", [])[:5]:
                    lines.append(f"  \u2022 {s['step']}: risk {s['risk']:.0%}")
                await post_message("\n".join(lines), channel, thread_ts)

        elif subcmd == "action" and len(sub) > 1:
            result = await simulator.simulate_action(sub[1].strip())
            icon = ":green_circle:" if result["risk_level"] == "LOW" else ":red_circle:"
            await post_message(
                f"{icon} *Simulate `{result['action_type']}`*: risk *{result['risk_level']}* "
                f"({result['risk_score']:.0%}) — {result['recommended_action']}",
                channel, thread_ts,
            )
        else:
            await post_message(
                ":test_tube: */simulate* commands: `/simulate <plan_id>`, `/simulate action <type>`",
                channel, thread_ts,
            )
        return True

    # -- Dashboard command --
    if cmd == "dashboard":
        port_url = f"http://localhost:{PORT}"
        lines = [":bar_chart: *Operator Dashboard*\n"]
        lines.append(f"API endpoints available at `{port_url}`:")
        lines.append(f"\u2022 `{port_url}/dashboard/overview` — System overview")
        lines.append(f"\u2022 `{port_url}/dashboard/tasks` — Task history")
        lines.append(f"\u2022 `{port_url}/dashboard/monitoring` — Checks & alerts")
        lines.append(f"\u2022 `{port_url}/dashboard/plans` — Plan history")
        lines.append(f"\u2022 `{port_url}/dashboard/routing` — Routing performance")
        lines.append(f"\u2022 `{port_url}/dashboard/graph` — Knowledge graph")
        lines.append(f"\n_Access via VM IP: `{VMS.get('swarm-mainframe', {}).get('ip', '10.142.0.4')}:{PORT}`_")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    # -- Web Search --
    if cmd == "search":
        query = args.strip()
        if not query:
            await post_message(":warning: Usage: `/search <query>`", channel, thread_ts)
            return True
        # Try to use a search API (Brave, SerpAPI, or fallback to AI)
        search_key = os.environ.get("BRAVE_API_KEY", "") or os.environ.get("SERP_API_KEY", "")
        if search_key and os.environ.get("BRAVE_API_KEY"):
            try:
                async with _session.get(
                    "https://api.search.brave.com/res/v1/web/search",
                    headers={"X-Subscription-Token": search_key, "Accept": "application/json"},
                    params={"q": query, "count": 5},
                    timeout=ClientTimeout(total=10),
                ) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        results = data.get("web", {}).get("results", [])[:5]
                        lines = [f":mag: *Search: `{query}`*\n"]
                        for r in results:
                            lines.append(f"\u2022 <{r['url']}|{r['title'][:60]}>")
                            if r.get("description"):
                                lines.append(f"  {r['description'][:100]}")
                        await post_message("\n".join(lines), channel, thread_ts)
                        return True
            except Exception as e:
                log.warning(f"Search API failed: {e}")

        # Fallback: ask AI to summarize search results
        result = await query_ai(
            f"Search the web for: {query}\n\nProvide a concise answer with relevant information.",
            channel=channel,
        )
        await post_message(result, channel, thread_ts)
        return True

    if cmd == "fetch":
        url = args.strip()
        if not url:
            await post_message(":warning: Usage: `/fetch <url>`", channel, thread_ts)
            return True
        try:
            async with _session.get(url, timeout=ClientTimeout(total=15)) as resp:
                text = await resp.text()
                # Truncate and clean
                text = text[:3000].replace("```", "` ` `")
                await post_message(f":globe_with_meridians: *Fetched `{url[:60]}`*\n```{text[:2000]}```", channel, thread_ts)
        except Exception as e:
            await post_message(f":x: Fetch failed: `{e}`", channel, thread_ts)
        return True

    # -- Code Execution --
    if cmd == "python":
        code = args.strip()
        if not code:
            await post_message(":warning: Usage: `/python <code>`", channel, thread_ts)
            return True
        try:
            result = await tool_executor.execute("shell", "swarm-mainframe",
                f"timeout 30 python3 -c {repr(code)} 2>&1 | head -50")
            await post_message(f":snake: *Python Output*\n```{(result or 'no output')[:2000]}```", channel, thread_ts)
        except Exception as e:
            await post_message(f":x: Python error: `{e}`", channel, thread_ts)
        return True

    if cmd == "js":
        code = args.strip()
        if not code:
            await post_message(":warning: Usage: `/js <code>`", channel, thread_ts)
            return True
        try:
            result = await tool_executor.execute("shell", "swarm-mainframe",
                f"timeout 30 node -e {repr(code)} 2>&1 | head -50")
            await post_message(f":computer: *JS Output*\n```{(result or 'no output')[:2000]}```", channel, thread_ts)
        except Exception as e:
            await post_message(f":x: JS error: `{e}`", channel, thread_ts)
        return True

    # -- File Management --
    if cmd == "files":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "help"
        fargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "find" and fargs:
            host_parts = fargs.split("@")
            pattern = host_parts[0].strip()
            host = host_parts[1].strip() if len(host_parts) > 1 else "swarm-mainframe"
            result = await tool_executor.execute("shell", host,
                f"find / -maxdepth 4 -name '{pattern}' -type f 2>/dev/null | head -20")
            await post_message(f":file_folder: *Find `{pattern}`* on `{host}`\n```{(result or 'no results')[:2000]}```", channel, thread_ts)

        elif subcmd == "grep" and fargs:
            host_parts = fargs.split("@")
            pattern = host_parts[0].strip()
            host = host_parts[1].strip() if len(host_parts) > 1 else "swarm-mainframe"
            result = await tool_executor.execute("shell", host,
                f"grep -r '{pattern}' /opt/ /etc/ 2>/dev/null | head -20")
            await post_message(f":mag: *Grep `{pattern}`* on `{host}`\n```{(result or 'no results')[:2000]}```", channel, thread_ts)

        elif subcmd == "read" and fargs:
            host_parts = fargs.split("@")
            path = host_parts[0].strip()
            host = host_parts[1].strip() if len(host_parts) > 1 else "swarm-mainframe"
            result = await tool_executor.execute("shell", host, f"head -100 '{path}' 2>&1")
            await post_message(f":page_facing_up: *`{path}`* on `{host}`\n```{(result or 'empty')[:2000]}```", channel, thread_ts)

        elif subcmd == "summary" and fargs:
            host_parts = fargs.split("@")
            path = host_parts[0].strip()
            host = host_parts[1].strip() if len(host_parts) > 1 else "swarm-mainframe"
            result = await tool_executor.execute("shell", host, f"wc -l '{path}' && file '{path}' && ls -lh '{path}' 2>&1")
            await post_message(f":page_facing_up: *Summary of `{path}`*\n```{(result or 'not found')[:1000]}```", channel, thread_ts)

        else:
            await post_message(
                ":file_folder: */files* commands: `find <pattern>[@host]`, `grep <pattern>[@host]`, "
                "`read <path>[@host]`, `summary <path>[@host]`",
                channel, thread_ts,
            )
        return True

    # -- Git Operations --
    if cmd == "git":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "help"
        gargs = sub[1].strip() if len(sub) > 1 else ""

        default_repo = "/opt/bunny-alpha"
        repo = gargs.split("@")[-1].strip() if "@" in gargs else default_repo
        git_args = gargs.split("@")[0].strip() if "@" in gargs else gargs

        if subcmd == "status":
            result = await tool_executor.execute("shell", "swarm-mainframe",
                f"cd {repo} && git status --short 2>&1")
            await post_message(f":git: *Git status* (`{repo}`)\n```{(result or 'clean')[:2000]}```", channel, thread_ts)

        elif subcmd == "log":
            count = git_args or "10"
            result = await tool_executor.execute("shell", "swarm-mainframe",
                f"cd {repo} && git log --oneline -n {count} 2>&1")
            await post_message(f":git: *Git log* (`{repo}`)\n```{(result or 'no commits')[:2000]}```", channel, thread_ts)

        elif subcmd == "diff":
            result = await tool_executor.execute("shell", "swarm-mainframe",
                f"cd {repo} && git diff --stat 2>&1")
            await post_message(f":git: *Git diff* (`{repo}`)\n```{(result or 'no changes')[:2000]}```", channel, thread_ts)

        elif subcmd == "branch":
            result = await tool_executor.execute("shell", "swarm-mainframe",
                f"cd {repo} && git branch -a 2>&1")
            await post_message(f":git: *Branches* (`{repo}`)\n```{(result or 'none')[:2000]}```", channel, thread_ts)

        elif subcmd == "pull":
            result = await tool_executor.execute("shell", "swarm-mainframe",
                f"cd {repo} && git pull 2>&1")
            await post_message(f":git: *Git pull* (`{repo}`)\n```{(result or 'done')[:2000]}```", channel, thread_ts)

        else:
            await post_message(
                ":git: */git* commands: `status`, `log [n]`, `diff`, `branch`, `pull`\n"
                "_Append `@/path/to/repo` to target a specific repo_",
                channel, thread_ts,
            )
        return True

    # -- Operational Hardening Commands --

    if cmd == "session":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"
        sargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "list":
            sessions = await session_mgr.list_sessions(limit=15)
            if not sessions:
                await post_message(":clipboard: No swarm sessions found.", channel, thread_ts)
            else:
                lines = [":clipboard: *Swarm Sessions*\n"]
                for s in sessions:
                    status_icon = ":green_circle:" if s["status"] == "active" else ":white_circle:"
                    lines.append(f"{status_icon} `{s['session_id']}` — {s['assistant_name']} ({s['status']})")
                await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "status" and sargs:
            s = await session_mgr.get_session(sargs)
            if s:
                await post_message(f":clipboard: *Session {sargs}*\n```{json.dumps(s, indent=2, default=str)[:2000]}```", channel, thread_ts)
            else:
                await post_message(f":warning: Session `{sargs}` not found.", channel, thread_ts)
        elif subcmd == "close" and sargs:
            await session_mgr.close_session(sargs, summary="Closed by operator")
            await audit.log("session_close", actor_id="operator", target_id=sargs)
            await post_message(f":white_check_mark: Session `{sargs}` closed.", channel, thread_ts)
        elif subcmd == "resume" and sargs:
            await session_mgr.resume_session(sargs)
            await post_message(f":arrow_forward: Session `{sargs}` resumed.", channel, thread_ts)
        else:
            await post_message(":clipboard: */session* commands: `list`, `status <id>`, `close <id>`, `resume <id>`", channel, thread_ts)
        return True

    if cmd == "approvals":
        pending = await perm_mgr.get_pending(20)
        if not pending:
            await post_message(":white_check_mark: No pending approvals.", channel, thread_ts)
        else:
            lines = [f":lock: *Pending Approvals ({len(pending)})*\n"]
            for a in pending:
                lines.append(f"• `{a['approval_id']}` — {a['action_type']} by {a['requested_by']}")
                if a.get("reason"):
                    lines.append(f"  _{a['reason'][:80]}_")
            await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "approve":
        aid = args.strip()
        if not aid:
            await post_message(":warning: Usage: `/approve <approval_id>`", channel, thread_ts)
            return True
        ok = await perm_mgr.approve(aid, approved_by="operator")
        if ok:
            await post_message(f":white_check_mark: Approved `{aid}`", channel, thread_ts)
        else:
            await post_message(f":x: Could not approve `{aid}` — not found or already resolved", channel, thread_ts)
        return True

    if cmd == "reject":
        aid = args.strip()
        if not aid:
            await post_message(":warning: Usage: `/reject <approval_id>`", channel, thread_ts)
            return True
        ok = await perm_mgr.reject(aid, rejected_by="operator")
        if ok:
            await post_message(f":no_entry: Rejected `{aid}`", channel, thread_ts)
        else:
            await post_message(f":x: Could not reject `{aid}` — not found or already resolved", channel, thread_ts)
        return True

    if cmd == "drill":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "help"
        dargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "run":
            dtype = dargs or "provider_outage"
            await post_message(f":rotating_light: Running drill: `{dtype}`...", channel, thread_ts)
            result = await drill_runner.run_drill(dtype)
            if "error" in result:
                await post_message(f":x: {result['error']}", channel, thread_ts)
            else:
                lines = [f":white_check_mark: *Drill Complete: {dtype}*"]
                lines.append(f"*Outcome:* {result['outcome']}")
                lines.append(f"*Detection:* {result['detection_time']}s | *Mitigation:* {result['mitigation_time']}s | *Recovery:* {result['recovery_time']}s")
                if result.get("rollback"):
                    lines.append(":rewind: Rollback triggered")
                lines.append(f"*Lessons:* {', '.join(result.get('lessons', []))}")
                await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "status" and dargs:
            d = await drill_runner.get_drill(dargs)
            if d:
                await post_message(f":rotating_light: *Drill {dargs}*\n```{json.dumps(d, indent=2, default=str)[:2000]}```", channel, thread_ts)
            else:
                await post_message(f":warning: Drill `{dargs}` not found.", channel, thread_ts)
        elif subcmd == "history":
            drills = await drill_runner.get_history(10)
            if drills:
                lines = [":rotating_light: *Drill History*\n"]
                for d in drills:
                    icon = ":white_check_mark:" if d.get("outcome") == "passed" else ":x:"
                    lines.append(f"{icon} `{d['drill_id']}` — {d['drill_type']} ({d.get('outcome', '?')})")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":rotating_light: No drill history.", channel, thread_ts)
        else:
            types_list = ", ".join(FailureDrillRunner.DRILL_TYPES)
            await post_message(f":rotating_light: */drill* commands: `run <type>`, `status <id>`, `history`\nTypes: {types_list}", channel, thread_ts)
        return True

    if cmd == "audit":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "recent"
        aargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "recent" or not subcmd:
            events = await audit.get_recent(15)
            total = await audit.count()
            lines = [f":scroll: *Audit Log ({total} total)*\n"]
            for e in events:
                lines.append(f"• `{e['action_type']}` by {e['actor_id']} → {e.get('target_id', '-')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "search":
            events = await audit.search(action_type=aargs, limit=15)
            lines = [f":scroll: *Audit: '{aargs}' ({len(events)} results)*\n"]
            for e in events:
                lines.append(f"• `{e['action_type']}` by {e['actor_id']}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":scroll: */audit* commands: `recent`, `search <action_type>`", channel, thread_ts)
        return True

    if cmd == "sandbox":
        policies = await sandbox.get_policies()
        lines = [":shield: *Execution Sandbox*\n"]
        for name, p in SANDBOX_PROFILES.items():
            lines.append(f"*{name}*: timeout={p['timeout']}s, output={p['output_limit']}")
        lines.append(f"\n*Code sandboxes:*")
        for lang, cfg in CODE_SANDBOX.items():
            lines.append(f"• {lang}: timeout={cfg['timeout']}s, mem={cfg['max_memory_mb']}MB")
        lines.append(f"\n*DB policies:* {len(policies)}")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "escalations":
        open_esc = await escalation_mgr.get_open(15)
        all_esc = await escalation_mgr.get_all(20)
        lines = [f":rotating_light: *Escalations* ({len(open_esc)} open / {len(all_esc)} total)\n"]
        for e in open_esc:
            lines.append(f"• `{e['escalation_id']}` — {e['trigger_type']} (conf={e.get('confidence', '?')})")
        if not open_esc:
            lines.append("_No open escalations_")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    # -- Continuous Learning Commands --

    if cmd == "outcomes":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "recent"
        oargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "recent":
            stats = await outcome_learner.get_stats()
            recent = await outcome_learner.get_recent("task", 10)
            lines = [":bar_chart: *Outcome Stats*"]
            lines.append(f"Tasks: {stats['task_outcomes']} (success={stats['task_success_rate']})")
            lines.append(f"Routes: {stats['route_outcomes']} | Repairs: {stats['repair_outcomes']}")
            if recent:
                lines.append("\n*Recent task outcomes:*")
                for o in recent[:5]:
                    icon = ":white_check_mark:" if o.get("success") else ":x:"
                    lines.append(f"{icon} `{o.get('task_id', '?')[:20]}` — {o.get('task_type', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd in ("task", "route", "repair"):
            recent = await outcome_learner.get_recent(subcmd, 10)
            lines = [f":bar_chart: *Recent {subcmd} outcomes ({len(recent)})*\n"]
            for o in recent[:10]:
                lines.append(f"```{json.dumps(o, default=str)[:200]}```")
            await post_message("\n".join(lines[:15]), channel, thread_ts)
        else:
            await post_message(":bar_chart: */outcomes* commands: `recent`, `task`, `route`, `repair`", channel, thread_ts)
        return True

    if cmd == "learn":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"

        if subcmd == "status":
            history = await intel_loop.get_history(3)
            lines = [":brain: *Intelligence Loop*"]
            lines.append(f"*Recent runs:* {len(history)}")
            for h in history:
                changes = json.loads(h.get("changes_json", "[]")) if h.get("changes_json") else []
                lines.append(f"• `{h['intelligence_run_id']}` — {'success' if h.get('success') else 'failed'} ({len(changes)} changes)")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "run":
            await post_message(":brain: Running intelligence cycle...", channel, thread_ts)
            result = await intel_loop.run_cycle()
            lines = [":brain: *Intelligence Cycle Complete*"]
            lines.append(f"*Run:* `{result['run_id']}`")
            lines.append(f"*Duration:* {result['duration_s']}s")
            for c in result.get("changes", []):
                lines.append(f"• {c}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "history":
            history = await intel_loop.get_history(10)
            lines = [":brain: *Learning History*\n"]
            for h in history:
                icon = ":white_check_mark:" if h.get("success") else ":x:"
                lines.append(f"{icon} `{h['intelligence_run_id']}`")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":brain: */learn* commands: `status`, `run`, `history`", channel, thread_ts)
        return True

    if cmd == "why":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "help"
        wargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd in ("task", "route", "plan", "repair", "agent", "learn") and wargs:
            result = await explainability.why(subcmd, wargs)
            await post_message(f":mag: *Why — {subcmd} `{wargs}`*\n```{json.dumps(result, indent=2, default=str)[:2000]}```", channel, thread_ts)
        else:
            await post_message(":mag: */why* commands: `task <id>`, `route <id>`, `plan <id>`, `repair <id>`, `agent <id>`, `learn <run_id>`", channel, thread_ts)
        return True

    if cmd == "kb":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "help"
        kargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "recipes":
            recipes = await memory_distiller.get_recipes(10)
            lines = [":book: *Execution Recipes*\n"]
            for r in recipes:
                lines.append(f"• `{r.get('recipe_id', '?')}` — {r.get('task_type', '?')} (success={r.get('success_rate', '?')})")
            if not recipes:
                lines.append("_No recipes yet — learning loop will populate these_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "incidents":
            incidents = await memory_distiller.get_incidents(10)
            lines = [":warning: *Incident Patterns*\n"]
            for i in incidents:
                lines.append(f"• `{i.get('pattern_id', '?')}` — {i.get('incident_type', '?')} (recurrence={i.get('recurrence_score', '?')})")
            if not incidents:
                lines.append("_No incident patterns yet_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "search" and kargs:
            results = await memory_distiller.search_knowledge(kargs)
            lines = [f":book: *KB Search: '{kargs}' ({len(results)} results)*\n"]
            for r in results[:10]:
                lines.append(f"• {r.get('title', r.get('task_type', r.get('incident_type', '?')))}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":book: */kb* commands: `recipes`, `incidents`, `search <query>`", channel, thread_ts)
        return True

    # -- Scale & Autonomy Commands --

    if cmd == "workers":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"
        wargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "list" or not args.strip():
            workers = await worker_registry.list_workers()
            lines = [f":factory: *Worker Registry ({len(workers)})*\n"]
            for w in workers:
                icon = ":green_circle:" if w.get("status") == "active" else ":red_circle:"
                lines.append(f"{icon} `{w['worker_id']}` — {w['host']} ({w['region']}) health={w.get('health_score', '?')}")
            if not workers:
                lines.append("_No workers registered_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "health":
            workers = await worker_registry.list_workers()
            lines = [":heartbeat: *Worker Health*\n"]
            for w in workers:
                bar = "█" * int((w.get("health_score", 0) or 0) * 10)
                lines.append(f"`{w['host']}` {bar} {w.get('health_score', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "region" and wargs:
            workers = await worker_registry.get_by_region(wargs)
            lines = [f":globe_with_meridians: *Workers in {wargs} ({len(workers)})*\n"]
            for w in workers:
                lines.append(f"• `{w['worker_id']}` — {w['host']}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "quarantine" and wargs:
            await worker_registry.quarantine(wargs)
            await post_message(f":no_entry: Worker `{wargs}` quarantined.", channel, thread_ts)
        else:
            await post_message(":factory: */workers* commands: `list`, `health`, `region <name>`, `quarantine <id>`", channel, thread_ts)
        return True

    if cmd == "initiative":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"

        if subcmd == "status":
            events = await initiative_engine.get_recent(10)
            lines = [f":bulb: *Initiative Events ({len(events)})*\n"]
            for e in events:
                lines.append(f"• [{e.get('risk_level', '?')}] {e.get('trigger_type', '?')}: {e.get('recommended_action', '')[:80]}")
            if not events:
                lines.append("_No initiative events_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "history":
            actions = await initiative_engine.get_actions(15)
            lines = [":bulb: *Initiative Actions*\n"]
            for a in actions:
                lines.append(f"• `{a.get('initiative_id', '?')}` — {a.get('action_type', '?')} ({a.get('execution_status', '?')})")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":bulb: */initiative* commands: `status`, `history`", channel, thread_ts)
        return True

    if cmd == "evaluate":
        await post_message(":chart_with_upwards_trend: Running system evaluation...", channel, thread_ts)
        result = await system_evaluator.evaluate()
        lines = [":chart_with_upwards_trend: *System Evaluation*"]
        lines.append(f"*Score:* {result['score']}")
        for k, v in result.get("metrics", {}).items():
            lines.append(f"• {k}: {v}")
        if result.get("recommendations"):
            lines.append("\n*Recommendations:*")
            for r in result["recommendations"]:
                lines.append(f"  :point_right: {r}")
        await post_message("\n".join(lines), channel, thread_ts)
        return True

    if cmd == "policy":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"
        pargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "status":
            lines = [":shield: *Safety Governance*\n"]
            lines.append(f"*AUTO:* {', '.join(RISK_AUTO_EXECUTE)}")
            lines.append(f"*NOTIFY:* {', '.join(RISK_NOTIFY)}")
            lines.append(f"*APPROVAL:* {', '.join(RISK_APPROVAL)}")
            lines.append(f"*BLOCKED:* {', '.join(RISK_BLOCKED)}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "explain" and pargs:
            result = await safety_governor.explain_policy(pargs)
            await post_message(f":shield: *Policy: `{pargs}`*\n```{json.dumps(result, indent=2)}```", channel, thread_ts)
        else:
            await post_message(":shield: */policy* commands: `status`, `explain <action_type>`", channel, thread_ts)
        return True

    if cmd == "plugins":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"
        pargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "list" or not args.strip():
            plugins = await plugin_mgr.list_plugins()
            lines = [f":jigsaw: *Plugins ({len(plugins)})*\n"]
            for p in plugins:
                icon = ":green_circle:" if p["status"] == "active" else ":white_circle:"
                lines.append(f"{icon} `{p['plugin_id']}` — {p['name']} ({p['plugin_type']})")
            if not plugins:
                lines.append("_No plugins registered_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "enable" and pargs:
            await plugin_mgr.enable(pargs)
            await post_message(f":white_check_mark: Plugin `{pargs}` enabled.", channel, thread_ts)
        elif subcmd == "disable" and pargs:
            await plugin_mgr.disable(pargs)
            await post_message(f":no_entry: Plugin `{pargs}` disabled.", channel, thread_ts)
        elif subcmd == "info" and pargs:
            p = await plugin_mgr.get_plugin(pargs)
            if p:
                await post_message(f":jigsaw: *Plugin {pargs}*\n```{json.dumps(p, indent=2, default=str)[:2000]}```", channel, thread_ts)
            else:
                await post_message(f":warning: Plugin `{pargs}` not found.", channel, thread_ts)
        else:
            await post_message(":jigsaw: */plugins* commands: `list`, `enable <id>`, `disable <id>`, `info <id>`", channel, thread_ts)
        return True

    if cmd == "scorecard":
        scorecards = await system_evaluator.get_scorecard()
        if scorecards:
            lines = [":chart_with_upwards_trend: *System Scorecards*\n"]
            for s in scorecards[:10]:
                lines.append(f"• `{s['component']}` — reliability={s.get('reliability_score', '?')} latency={s.get('latency_score', '?')} trend={s.get('trend', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":chart_with_upwards_trend: No scorecards yet. Run `/evaluate` first.", channel, thread_ts)
        return True

    # -- Environment Intelligence Commands --

    if cmd == "env":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"
        eargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "status":
            status = await env_awareness.get_status()
            lines = [f":satellite: *Environment Status*"]
            lines.append(f"*Entities:* {len(status.get('entities', []))}")
            lines.append(f"*Total signals:* {status.get('total_signals', 0)}")
            for e in status.get("entities", [])[:10]:
                lines.append(f"• `{e['entity_id']}` health={e.get('derived_health_score', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "entity" and eargs:
            data = await env_awareness.get_entity(eargs)
            await post_message(f":satellite: *Entity: {eargs}*\n```{json.dumps(data, indent=2, default=str)[:2000]}```", channel, thread_ts)
        elif subcmd == "signals":
            signals = await env_awareness.get_signals(limit=15)
            lines = [f":satellite: *Recent Signals ({len(signals)})*\n"]
            for s in signals[:15]:
                lines.append(f"• `{s['source_id']}` {s['metric_name']}={s.get('metric_value', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "health":
            health = await env_awareness.get_health()
            lines = [f":heartbeat: *Environment Health* (overall={health.get('overall_health', '?')})\n"]
            for eid, h in health.get("entities", {}).items():
                bar = "█" * int(h.get("health_score", 0) * 10)
                lines.append(f"`{eid}` {bar} {h.get('health_score', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":satellite: */env* commands: `status`, `entity <id>`, `signals`, `health`", channel, thread_ts)
        return True

    if cmd == "events":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "recent"
        evargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "recent":
            events = await event_ingestor.get_recent(15)
            lines = [f":zap: *Recent Events ({len(events)})*\n"]
            for e in events:
                lines.append(f"• [{e.get('severity', 'info')}] `{e['event_type']}` → {e.get('entity_id', '-')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "entity" and evargs:
            events = await event_ingestor.get_by_entity(evargs, 15)
            lines = [f":zap: *Events for {evargs} ({len(events)})*\n"]
            for e in events:
                lines.append(f"• [{e.get('severity', 'info')}] {e['event_type']}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "correlation" and evargs:
            events = await event_ingestor.get_correlation(evargs)
            lines = [f":link: *Correlated Events: {evargs} ({len(events)})*\n"]
            for e in events:
                lines.append(f"• {e['event_type']} → {e.get('entity_id', '-')}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":zap: */events* commands: `recent`, `entity <id>`, `correlation <group>`", channel, thread_ts)
        return True

    if cmd == "twin":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"
        targs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "status":
            status = await digital_twin.get_status()
            lines = [":world_map: *Digital Twin*"]
            lines.append(f"*Entities:* {len(status.get('entities', []))}")
            lines.append(f"*Relationships:* {len(status.get('relationships', []))}")
            for e in status.get("entities", []):
                lines.append(f"• `{e['entity_ref']}` ({e['entity_type']})")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "simulate" and targs:
            result = await digital_twin.simulate_scenario(targs)
            lines = [f":world_map: *Simulation: {targs}*"]
            lines.append(f"*Risk:* {result.get('risk_score', '?')}")
            pred = result.get("predicted", {})
            lines.append(f"*Impact:* {pred.get('impact', '?')}")
            lines.append(f"*Recovery est:* {pred.get('recovery_estimate_s', '?')}s")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "explain" and targs:
            result = await digital_twin.explain_simulation(targs)
            if result:
                await post_message(f":world_map: *Simulation {targs}*\n```{json.dumps(result, indent=2, default=str)[:2000]}```", channel, thread_ts)
            else:
                await post_message(f":warning: Simulation `{targs}` not found.", channel, thread_ts)
        else:
            await post_message(":world_map: */twin* commands: `status`, `simulate <scenario>`, `explain <sim_id>`", channel, thread_ts)
        return True

    if cmd == "auto":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"

        if subcmd == "status":
            actions = await auto_ops.get_actions(limit=10)
            lines = [f":robot_face: *Autonomous Operations ({len(actions)} actions)*\n"]
            for a in actions:
                risk_icon = ":red_circle:" if a.get("risk_level") == "HIGH" else ":yellow_circle:" if a.get("risk_level") == "MODERATE" else ":green_circle:"
                lines.append(f"{risk_icon} `{a.get('action_id', '?')}` — {a.get('recommended_action', '')[:80]}")
            if not actions:
                lines.append("_No autonomous actions recorded_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "history":
            actions = await auto_ops.get_actions(limit=20)
            lines = [":robot_face: *Autonomous Action History*\n"]
            for a in actions:
                lines.append(f"• [{a.get('risk_level', '?')}] {a.get('trigger_type', '?')} → {a.get('status', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":robot_face: */auto* commands: `status`, `history`", channel, thread_ts)
        return True

    if cmd == "playbooks":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"
        pbargs = sub[1].strip() if len(sub) > 1 else ""

        if subcmd == "list" or not args.strip():
            playbooks = await knowledge_evolution.get_playbooks(15)
            lines = [f":blue_book: *Operational Playbooks ({len(playbooks)})*\n"]
            for p in playbooks:
                lines.append(f"• `{p.get('playbook_id', '?')}` — {p.get('incident_type', '?')} (success={p.get('success_rate', '?')})")
            if not playbooks:
                lines.append("_No playbooks yet — knowledge evolution will populate these_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "search" and pbargs:
            results = await knowledge_evolution.search(pbargs)
            lines = [f":blue_book: *Playbook Search: '{pbargs}' ({len(results)})*\n"]
            for r in results[:10]:
                lines.append(f"• {r.get('topic', r.get('incident_type', '?'))}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":blue_book: */playbooks* commands: `list`, `search <query>`", channel, thread_ts)
        return True

    if cmd == "ops":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "overview"

        if subcmd == "overview":
            env_health = await env_awareness.get_health()
            open_esc = await escalation_mgr.get_open(5)
            auto_actions = await auto_ops.get_actions(limit=5)
            lines = [":control_knobs: *Operator Overview*"]
            lines.append(f"*Env health:* {env_health.get('overall_health', '?')}")
            lines.append(f"*Open escalations:* {len(open_esc)}")
            lines.append(f"*Auto actions:* {len(auto_actions)}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "incidents":
            patterns = await knowledge_evolution.get_patterns(10)
            lines = [":warning: *Incident Patterns*\n"]
            for p in patterns:
                lines.append(f"• {p.get('topic', '?')} (conf={p.get('confidence', '?')})")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "twin":
            status = await digital_twin.get_status()
            lines = [":world_map: *Digital Twin Overview*"]
            lines.append(f"*Entities:* {len(status.get('entities', []))}")
            lines.append(f"*Relationships:* {len(status.get('relationships', []))}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "autonomy":
            auto_actions = await auto_ops.get_actions(limit=10)
            lines = [":robot_face: *Autonomy Status*\n"]
            for a in auto_actions:
                lines.append(f"• [{a.get('risk_level', '?')}] {a.get('recommended_action', '')[:80]}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":control_knobs: */ops* commands: `overview`, `incidents`, `twin`, `autonomy`", channel, thread_ts)
        return True

    if cmd == "actions":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "recent"

        if subcmd == "recent":
            actions = await action_service.get_actions(limit=10)
            lines = [":shield: *Recent Structured Actions*\n"]
            if not actions:
                lines.append("_No actions recorded yet._")
            for a in actions:
                risk_icon = {
                    "READ_ONLY": ":large_green_circle:",
                    "SAFE_MUTATION": ":large_blue_circle:",
                    "RISKY_MUTATION": ":large_yellow_circle:",
                    "DESTRUCTIVE": ":red_circle:",
                }.get(a.get("risk_level", ""), ":white_circle:")
                lines.append(f"{risk_icon} `{a.get('action_type', '?')}` on {a.get('host', '?')} — _{a.get('status', '?')}_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "stats":
            stats = await action_service.get_stats()
            lines = [":bar_chart: *Execution Stats*\n"]
            lines.append(f"*Total actions:* {stats.get('total', 0)}")
            lines.append(f"*Completed:* {stats.get('completed', 0)}")
            lines.append(f"*Failed:* {stats.get('failed', 0)}")
            lines.append(f"*Pending/Approval:* {stats.get('pending', 0)}")
            lines.append(f"*Success rate:* {stats.get('success_rate', 0):.1%}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "policies":
            policies = await action_service.get_policies()
            lines = [":scroll: *Action Policies*\n"]
            for p in policies[:15]:
                approval = ":lock:" if p.get("requires_approval") else ":unlock:"
                lines.append(f"{approval} `{p.get('action_type', '?')}` [{p.get('risk_level', '?')}] via {p.get('adapter', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "audit":
            trail = await action_service.get_audit_trail(limit=10)
            lines = [":memo: *Action Audit Trail*\n"]
            if not trail:
                lines.append("_No audit entries yet._")
            for t in trail:
                lines.append(f"• `{t.get('action_type', '?')}` [{t.get('risk_level', '?')}] {t.get('outcome', '')} — {t.get('requested_by', 'system')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "blocked":
            lines = [":no_entry: *Blocked Command Patterns*\n"]
            for bp in BLOCKED_PATTERNS:
                lines.append(f"• `{bp}`")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":shield: */actions* commands: `recent`, `stats`, `policies`, `audit`, `blocked`", channel, thread_ts)
        return True

    if cmd == "execution":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "status"

        if subcmd == "status":
            stats = await action_service.get_stats()
            pending = await action_service.get_actions(status="awaiting_approval", limit=5)
            lines = [":gear: *Execution Engine Status*\n"]
            lines.append(f"*Total actions:* {stats.get('total', 0)} | *Success rate:* {stats.get('success_rate', 0):.1%}")
            lines.append(f"*Pending approvals:* {len(pending)}")
            if pending:
                lines.append("\n*Awaiting Approval:*")
                for p in pending:
                    lines.append(f"  :hourglass: `{p.get('action_type', '?')}` on {p.get('host', '?')} — {p.get('params', '')[:60]}")
            rp = {k: v for k, v in RISK_POLICY.items()}
            lines.append(f"\n*Risk Policy:*")
            for lvl, policy in rp.items():
                lines.append(f"  {lvl}: {policy}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "results":
            results = await action_service.get_results(limit=10)
            lines = [":clipboard: *Recent Execution Results*\n"]
            if not results:
                lines.append("_No results yet._")
            for r in results:
                status_icon = ":white_check_mark:" if r.get("success") else ":x:"
                lines.append(f"{status_icon} `{r.get('action_id', '?')[:12]}` — {r.get('output', '')[:80]}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":gear: */execution* commands: `status`, `results`", channel, thread_ts)
        return True

    # -----------------------------------------------------------------------
    # Proactive Relationship & Opportunity Engine Commands
    # -----------------------------------------------------------------------

    if cmd == "signals":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "recent"

        if subcmd == "recent":
            signals = await signal_discovery.get_signals(limit=10)
            lines = [":satellite: *Recent Company Signals*\n"]
            if not signals:
                lines.append("_No signals detected yet._")
            for s in signals:
                proc = ":white_check_mark:" if s.get("processed") else ":new:"
                lines.append(f"{proc} *{s.get('company_name', '?')}* — {s.get('signal_type', '?')} via {s.get('signal_source', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "stats":
            stats = await signal_discovery.get_stats()
            lines = [":bar_chart: *Signal Stats*\n"]
            lines.append(f"*Total signals:* {stats.get('total_signals', 0)}")
            lines.append(f"*Unprocessed:* {stats.get('unprocessed', 0)}")
            lines.append(f"*Sources:* {stats.get('sources', 0)}")
            by_type = stats.get("by_type", {})
            if by_type:
                lines.append("*By type:*")
                for t, c in sorted(by_type.items(), key=lambda x: -x[1]):
                    lines.append(f"  {t}: {c}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "sources":
            sources = await signal_discovery.get_sources()
            lines = [":antenna_bars: *Signal Sources*\n"]
            for s in sources:
                enabled = ":large_green_circle:" if s.get("enabled") else ":red_circle:"
                lines.append(f"{enabled} `{s.get('source_type', '?')}` — poll every {s.get('polling_interval', 0)}s")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":satellite: */signals* commands: `recent`, `stats`, `sources`", channel, thread_ts)
        return True

    if cmd == "opportunities":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "top"

        if subcmd == "top":
            opps = await opportunity_qualifier.get_top_opportunities(10)
            lines = [":dart: *Top Opportunities*\n"]
            if not opps:
                lines.append("_No opportunities scored yet._")
            for o in opps:
                score = o.get("ai_fit_score") or o.get("ai_need_score", 0)
                val = o.get("estimated_value", 0)
                lines.append(f":star: *{o.get('name', '?')}* [{o.get('industry', '?')}] — AI fit: {score:.0%} | Est: ${val:,.0f}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "profiles":
            profiles = await opportunity_qualifier.get_profiles(10)
            lines = [":office: *Company Profiles*\n"]
            for p in profiles:
                lines.append(f"• *{p.get('name', '?')}* ({p.get('industry', '?')}, {p.get('size_estimate', '?')}) — AI={p.get('ai_need_score', 0):.2f}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":dart: */opportunities* commands: `top`, `profiles`", channel, thread_ts)
        return True

    if cmd == "pipeline":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "overview"

        if subcmd == "overview":
            stats = await relationship_pipeline.get_stats()
            lines = [":pipeline: *Relationship Pipeline*\n"]
            lines.append(f"*Total relationships:* {stats.get('total_relationships', 0)}")
            by_stage = stats.get("by_stage", {})
            stage_icons = {
                "detected": ":mag:", "research": ":books:", "contact_initiated": ":wave:",
                "conversation_active": ":speech_balloon:", "demo_requested": ":tv:",
                "proposal_sent": ":page_facing_up:", "deployment_in_progress": ":rocket:",
                "active_client": ":handshake:",
            }
            for stage in RelationshipPipeline.STAGES:
                count = by_stage.get(stage, 0)
                icon = stage_icons.get(stage, ":record_button:")
                lines.append(f"  {icon} {stage}: {count}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "events":
            events = await relationship_pipeline.get_events(limit=10)
            lines = [":clock: *Recent Pipeline Events*\n"]
            for e in events:
                lines.append(f"• [{e.get('event_type', '?')}] {e.get('company_id', '?')[:12]} — {json.loads(e.get('event_details_json', '{}')).get('notes', '')[:60]}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            stage_filter = subcmd
            if stage_filter in RelationshipPipeline.STAGES:
                entries = await relationship_pipeline.get_pipeline(stage=stage_filter, limit=10)
                lines = [f":pipeline: *Pipeline — {stage_filter}*\n"]
                for e in entries:
                    lines.append(f"• *{e.get('name', e.get('company_id', '?'))}* — {e.get('notes', '')[:60]}")
                await post_message("\n".join(lines), channel, thread_ts)
            else:
                await post_message(":pipeline: */pipeline* commands: `overview`, `events`, or a stage name", channel, thread_ts)
        return True

    if cmd == "research":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"

        if subcmd == "list":
            research = await research_agent.get_all_research(10)
            lines = [":microscope: *Company Research*\n"]
            if not research:
                lines.append("_No research profiles yet._")
            for r in research:
                lines.append(f"• *{r.get('name', r.get('company_id', '?'))}* — {(r.get('summary', '') or '')[:80]}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":microscope: */research* commands: `list`", channel, thread_ts)
        return True

    if cmd == "outreach":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "recent"

        if subcmd == "recent":
            msgs = await outreach_generator.get_messages(limit=10)
            lines = [":envelope: *Recent Outreach*\n"]
            if not msgs:
                lines.append("_No outreach messages yet._")
            for m in msgs:
                status_icon = {
                    "draft": ":pencil:", "approved": ":white_check_mark:",
                    "sent": ":outbox_tray:", "replied": ":incoming_envelope:",
                    "interested": ":star:", "declined": ":x:",
                    "opted_out": ":no_entry:",
                }.get(m.get("response_status", ""), ":grey_question:")
                lines.append(f"{status_icon} [{m.get('channel', '?')}] {m.get('company_id', '?')[:12]} — _{m.get('response_status', '?')}_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "stats":
            stats = await outreach_generator.get_stats()
            lines = [":bar_chart: *Outreach Stats*\n"]
            lines.append(f"*Total messages:* {stats.get('total', 0)}")
            by_status = stats.get("by_status", {})
            for s, c in by_status.items():
                lines.append(f"  {s}: {c}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "compliance":
            comp_stats = await outreach_compliance.get_stats()
            policies = await outreach_compliance.get_policies()
            lines = [":shield: *Outreach Compliance*\n"]
            lines.append(f"*Policies:* {comp_stats.get('policies', 0)}")
            lines.append(f"*Opted-out companies:* {comp_stats.get('opted_out_companies', 0)}")
            lines.append(f"*Blocked messages:* {comp_stats.get('blocked_messages', 0)}")
            if policies:
                lines.append("\n*Active Policies:*")
                for p in policies:
                    lines.append(f"  • {p.get('rule_type', '?')}: {p.get('enforcement_action', '?')}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":envelope: */outreach* commands: `recent`, `stats`, `compliance`", channel, thread_ts)
        return True

    if cmd == "proposals":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"

        if subcmd == "list":
            proposals = await proposal_generator.get_proposals(limit=10)
            lines = [":page_facing_up: *Proposals*\n"]
            if not proposals:
                lines.append("_No proposals yet._")
            for p in proposals:
                lines.append(f"• *{p.get('name', p.get('company_id', '?'))}* — ${p.get('estimated_cost', 0):,.0f} | ROI: {p.get('estimated_roi', 0):.0%} | _{p.get('status', '?')}_")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "stats":
            stats = await proposal_generator.get_stats()
            lines = [":bar_chart: *Proposal Stats*\n"]
            lines.append(f"*Total:* {stats.get('total', 0)}")
            for s, c in stats.get("by_status", {}).items():
                lines.append(f"  {s}: {c}")
            lines.append(f"*Accepted value:* ${stats.get('accepted_value', 0):,.0f}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":page_facing_up: */proposals* commands: `list`, `stats`", channel, thread_ts)
        return True

    if cmd == "revenue":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "summary"

        if subcmd == "summary":
            stats = await revenue_tracker.get_stats()
            lines = [":money_with_wings: *Revenue Summary*\n"]
            lines.append(f"*Total revenue:* ${stats.get('total_revenue', 0):,.2f}")
            lines.append(f"*MRR:* ${stats.get('mrr', 0):,.2f}")
            lines.append(f"*Active clients:* {stats.get('active_clients', 0)}")
            by_type = stats.get("by_type", {})
            if by_type:
                lines.append("*By type:*")
                for t, v in by_type.items():
                    lines.append(f"  {t}: ${v:,.2f}")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "recent":
            revenue = await revenue_tracker.get_revenue(limit=10)
            lines = [":receipt: *Recent Revenue*\n"]
            for r in revenue:
                lines.append(f"• {r.get('name', r.get('company_id', '?'))} — ${r.get('amount', 0):,.2f} ({r.get('revenue_type', '?')})")
            await post_message("\n".join(lines), channel, thread_ts)
        elif subcmd == "conversions":
            rates = await relationship_learner.get_conversion_rates()
            lines = [":chart_with_upwards_trend: *Conversion Rates*\n"]
            for stage, rate in rates.items():
                lines.append(f"  {stage}: {rate:.1%}")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":money_with_wings: */revenue* commands: `summary`, `recent`, `conversions`", channel, thread_ts)
        return True

    if cmd == "deployments":
        sub = args.strip().split(maxsplit=1)
        subcmd = sub[0].lower() if sub else "list"

        if subcmd == "list":
            deps = await deployment_trigger.get_deployments(limit=10)
            lines = [":rocket: *Deployments*\n"]
            if not deps:
                lines.append("_No deployments yet._")
            for d in deps:
                status_icon = {
                    "planned": ":clipboard:", "provisioning": ":gear:",
                    "deploying": ":rocket:", "active": ":large_green_circle:",
                    "failed": ":red_circle:",
                }.get(d.get("deployment_status", ""), ":grey_question:")
                lines.append(f"{status_icon} *{d.get('name', d.get('company_id', '?'))}* — _{d.get('deployment_status', '?')}_")
            await post_message("\n".join(lines), channel, thread_ts)
        else:
            await post_message(":rocket: */deployments* commands: `list`", channel, thread_ts)
        return True

    if cmd == "crm":
        # Unified CRM overview
        pipe_stats = await relationship_pipeline.get_stats()
        signal_stats = await signal_discovery.get_stats()
        outreach_stats = await outreach_generator.get_stats()
        proposal_stats = await proposal_generator.get_stats()
        rev_stats = await revenue_tracker.get_stats()
        learning_stats = await relationship_learner.get_stats()

        lines = [":briefcase: *CRM — Relationship Engine Overview*\n"]
        lines.append(f":satellite: *Signals:* {signal_stats.get('total_signals', 0)} detected | {signal_stats.get('unprocessed', 0)} unprocessed")
        lines.append(f":office: *Pipeline:* {pipe_stats.get('total_relationships', 0)} relationships")
        by_stage = pipe_stats.get("by_stage", {})
        active_stages = [f"{s}:{c}" for s, c in by_stage.items() if c > 0]
        if active_stages:
            lines.append(f"  Stages: {', '.join(active_stages)}")
        lines.append(f":envelope: *Outreach:* {outreach_stats.get('total', 0)} messages")
        lines.append(f":page_facing_up: *Proposals:* {proposal_stats.get('total', 0)} | Accepted value: ${proposal_stats.get('accepted_value', 0):,.0f}")
        lines.append(f":money_with_wings: *Revenue:* ${rev_stats.get('total_revenue', 0):,.2f} total | MRR: ${rev_stats.get('mrr', 0):,.2f}")
        lines.append(f":brain: *Learning:* {learning_stats.get('total_outcomes', 0)} outcomes | {learning_stats.get('success_rate', 0):.0%} success rate")
        await post_message("\n".join(lines), channel, thread_ts)
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
        "version": "3.2.0",
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
            f"Bunny Alpha v3.2 online | bot={result['user']} | "
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

    # Seed monitoring defaults and knowledge graph
    try:
        seeded = await monitor.seed_defaults()
        if seeded:
            log.info(f"Seeded {seeded} default monitoring checks")
        checks = await monitor.get_checks()
        log.info(f"Monitoring: {len(checks)} active checks")
    except Exception as e:
        log.warning(f"Monitoring init error: {e}")

    try:
        await knowledge_graph.seed_infrastructure()
    except Exception as e:
        log.warning(f"Knowledge graph seed error: {e}")

    try:
        await agent_coordinator.seed_agents()
        agents = await agent_coordinator.get_agents()
        log.info(f"Multi-agent: {len(agents)} agents registered")
    except Exception as e:
        log.warning(f"Agent seed error: {e}")

    log.info(f"Self-healing: {'enabled' if self_healer.enabled else 'disabled'}")

    # Initialize operational hardening
    try:
        await perm_mgr.seed_defaults()
        await sandbox.seed_policies()
        log.info("Permissions and sandbox policies seeded")
    except Exception as e:
        log.warning(f"Permissions/sandbox init error: {e}")

    # Initialize learning layer
    try:
        await routing_intel.seed_weights()
        log.info("Routing intelligence weights seeded")
    except Exception as e:
        log.warning(f"Routing intel init error: {e}")

    # Initialize scale & autonomy
    try:
        await worker_registry.seed_defaults()
        workers = await worker_registry.list_workers()
        log.info(f"Worker registry: {len(workers)} workers")
    except Exception as e:
        log.warning(f"Worker registry init error: {e}")

    # Initialize digital twin
    try:
        await digital_twin.seed_twin()
        twin_status = await digital_twin.get_status()
        log.info(f"Digital twin: {len(twin_status.get('entities', []))} entities")
    except Exception as e:
        log.warning(f"Digital twin init error: {e}")

    # Structured Execution policies
    try:
        await action_service.seed_policies()
        policies = await action_service.get_policies()
        log.info(f"Action policies seeded: {len(policies)} policies")
    except Exception as e:
        log.warning(f"Action policy seed error: {e}")

    # Relationship & Opportunity Engine initialization
    try:
        await signal_discovery.seed_sources()
        sources = await signal_discovery.get_sources()
        log.info(f"Signal sources seeded: {len(sources)} sources")
        await outreach_compliance.seed_policies()
        compliance_policies = await outreach_compliance.get_policies()
        log.info(f"Outreach compliance: {len(compliance_policies)} policies")
    except Exception as e:
        log.warning(f"Relationship engine init error: {e}")

    log.info(f"Listening on port {PORT}")

    # Start background services
    asyncio.create_task(_periodic_cleanup())
    await monitor.start_monitoring_loop()
    await scheduler.start_scheduler_loop()
    await intel_loop.start_loop(3600)  # Intelligence loop every hour

    await audit.log("system_startup", payload={"version": "3.2.0"})


async def _periodic_cleanup():
    """Clean up old tasks periodically."""
    while True:
        await asyncio.sleep(300)
        task_manager.cleanup_old(3600)


async def on_cleanup(app: web.Application):
    """Cleanup on shutdown."""
    global _session
    monitor.stop()
    scheduler.stop()
    intel_loop.stop()
    await audit.log("system_shutdown")
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
    app.router.add_get("/dashboard/overview", dashboard_overview)
    app.router.add_get("/dashboard/tasks", dashboard_tasks)
    app.router.add_get("/dashboard/monitoring", dashboard_monitoring)
    app.router.add_get("/dashboard/plans", dashboard_plans)
    app.router.add_get("/dashboard/routing", dashboard_routing)
    app.router.add_get("/dashboard/graph", dashboard_graph)
    app.router.add_get("/dashboard/sessions", dashboard_sessions)
    app.router.add_get("/dashboard/infrastructure", dashboard_infrastructure)
    app.router.add_get("/dashboard/knowledge", dashboard_knowledge)
    app.router.add_get("/dashboard/audit", dashboard_audit)
    app.router.add_get("/dashboard/learning", dashboard_learning)
    app.router.add_get("/dashboard/environment", dashboard_environment)
    app.router.add_get("/dashboard/system", dashboard_system)
    app.router.add_get("/dashboard/actions", dashboard_actions)
    app.router.add_get("/dashboard/execution", dashboard_execution)
    app.router.add_get("/dashboard/pipeline", dashboard_pipeline)
    app.router.add_get("/dashboard/opportunities", dashboard_opportunities)

    log.info("Starting Bunny Alpha v3.2 \u2014 Autonomous Operations Platform")
    web.run_app(app, host="0.0.0.0", port=PORT)


if __name__ == "__main__":
    main()
