# BSA-00 — Swarm Boundary & Source of Truth

**System:** Calculus AI Platform
**Authority:** BUNNY Core
**Directive Type:** Foundational Infrastructure Constraint
**Priority:** HIGHEST — overrides all other directives on scope questions

---

## 1. Rule

The Swarm operates **exclusively** on three infrastructure nodes:

| Node | IP | Role |
|------|----|------|
| **swarm-mainframe** | 34.148.140.31 | Source of truth — orchestration, agents, directive store, memory |
| **swarm-gpu** | 35.227.111.161 | Inference engine — model serving only |
| **fc-ai-portal** | 34.139.78.75 | API hub — LLM keys, external connectors, user-facing APIs |

**swarm-mainframe is the source of truth for all swarm state.**

---

## 2. What This Means

### 2.1 Code Execution
- All swarm logic runs on one of the three nodes above
- No swarm code executes locally on developer machines
- No swarm state lives outside these three nodes

### 2.2 Development Workflow
```
Developer (Claude Local)
    → write code
    → git push to GitHub
    → node pulls from GitHub
    → executes on node
```
Claude Local is a development workspace only — never a runtime target.

### 2.3 State Ownership
| State Type | Owner |
|-----------|-------|
| Directive store (SQLite) | swarm-mainframe |
| Agent memory | swarm-mainframe |
| Model weights | swarm-gpu |
| LLM API keys | fc-ai-portal |
| Git source of truth | GitHub (financecommander org) |

### 2.4 Inter-Node Communication
- swarm-mainframe → swarm-gpu: inference requests (internal GCP network)
- swarm-mainframe → fc-ai-portal: key fetch, API dispatch (internal GCP network)
- External signals (Signal, Twilio, Telegram) → fc-ai-portal or swarm-mainframe only

---

## 3. Global Task Reach

The swarm **executes tasks globally** — web research, API calls, data collection,
external service integrations, calls, emails, deployments, and any other outbound
work can reach any target worldwide.

The boundary rule governs where the swarm **lives and stores state**, not where
it **acts**. Orchestration stays on mainframe; task execution reaches everywhere.

```
swarm-mainframe (orchestrates)
    → dispatches tasks to workers
        → workers act globally (any API, any endpoint, any service)
        → results returned to mainframe
    → state updated on mainframe
```

---

## 4. Prohibited Patterns

- Storing swarm state on developer machines
- Running swarm agents outside the three nodes
- Hardcoding external IPs instead of GCP internal hostnames
- Any node writing persistent state to another node's domain

---

## 4. Directive Activation

**BSA-00 is active. This is a foundational constraint.**

BUNNY Core enforces this boundary. All other BSA directives operate within it.
