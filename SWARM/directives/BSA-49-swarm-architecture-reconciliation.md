# BSA-49 — SWARM Architecture Reconciliation

**System:** SWARM  
**Authority:** CONTROL PLANE ONLY  
**Directive Type:** Canon Reconciliation Directive  
**Classification:** Internal Architecture Canon

---

## 1. Executive Decision

SWARM uses a **5-plane canonical architecture** as the sole architectural source of truth:

1. `RUNTIME`
2. `DATA COLLECTION`
3. `MONITORING`
4. `LEARNING`
5. `CONTROL PLANE`

All prior architecture models are retained only as **secondary views**:

- `SWARM-ARCH-LAYERS-32` is an **implementation/layer decomposition view**
- `BSA-01/BSA-02` is a **capability taxonomy and historical vocabulary**

Neither legacy model may define separate write authority, competing planes, or alternate system sovereignty.

---

## 2. Canonical 5-Plane Definition

### 2.1 RUNTIME

RUNTIME executes work in real time.

It includes:
- ingress
- agent surfaces
- request parsing
- runtime reasoning
- provider routing
- skill routing
- tool execution
- baseline fallback behavior

RUNTIME may execute only within control-plane-approved policy.

### 2.2 DATA COLLECTION

DATA COLLECTION captures comparative evidence and training signals.

It includes:
- shadow routing
- provider sweeps
- skill-routing comparisons
- baseline vs neural disagreement capture
- labeled training row generation
- oracle-source capture

### 2.3 MONITORING

MONITORING observes the entire system and detects degradation.

It includes:
- latency monitoring
- readiness monitoring
- health monitoring
- drift detection
- rollback recommendations
- confidence distribution analysis
- tail-latency and stall detection

### 2.4 LEARNING

LEARNING trains and evaluates models.

It includes:
- GBM routing
- neural routing
- calibration
- offline eval
- shadow eval
- advisory recommendations

LEARNING is advisory only.

### 2.5 CONTROL PLANE

CONTROL PLANE governs the system.

It includes:
- rollout state
- thresholds
- policy
- allowlists
- authority gating
- promotion approval
- rollback enforcement

CONTROL PLANE is the only plane allowed to change runtime behavior.

---

## 3. Reconciliation of SWARM-ARCH-LAYERS-32

The 10-layer model remains useful as a runtime anatomy view, but it is not a competing architecture.

| Legacy Layer | Legacy Name | Canonical Plane | Canonical Status |
|---|---|---|---|
| L0 | External Interfaces | RUNTIME | subordinate view |
| L1 | Agent Surface | RUNTIME | subordinate view |
| L2 | Command / Ingress | RUNTIME | subordinate view |
| L3 | Control Plane | CONTROL PLANE | canonical only at plane level |
| L4 | Cognition Plane | RUNTIME | subordinate view |
| L5 | Memory / Context Plane | infrastructure / data substrate | not a top-level plane |
| L6 | Execution Plane | RUNTIME | subordinate view |
| L7 | Verification / Governance | CONTROL PLANE + MONITORING | subordinate view |
| L8 | Maintenance / Self-Healing | CONTROL PLANE + MONITORING | subordinate view |
| L9 | Data / Infrastructure | infrastructure substrate | not a plane |

### Non-canonical assumptions in ARCH-32

- verification may not act as an independent authority plane
- maintenance may not act as an independent authority plane
- cognition is not a top-level plane
- memory is not a top-level plane

---

## 4. Reconciliation of BSA-01 / BSA-02

The 6-plane model remains useful as a vocabulary for capabilities, but not as the governing architecture.

| Legacy Plane | Canonical Plane | Canonical Status |
|---|---|---|
| Security Plane | cross-cutting governance concern under CONTROL PLANE | subordinate capability |
| Control Plane | CONTROL PLANE | canonical only at control level |
| Cognition Plane | RUNTIME | subordinate capability |
| Execution Plane | RUNTIME | subordinate capability |
| Memory Plane | infrastructure / state substrate | not a top-level plane |
| Collective Intelligence Plane | LEARNING + DATA COLLECTION | subordinate capability |

### Non-canonical assumptions in BSA-01 / BSA-02

- security is not a separate plane with parallel authority
- cognition is not a separate plane
- execution is not a separate plane
- memory is not a separate plane
- collective intelligence does not replace LEARNING and DATA COLLECTION

---

## 5. Conflict Resolution Rules

If architectural documents conflict, apply these rules:

1. If a model omits `DATA COLLECTION` or `MONITORING`, it is incomplete and non-canonical.
2. If a model introduces additional write-authority planes, it is non-canonical.
3. If a model allows LEARNING to directly change runtime behavior, it is non-canonical.
4. If a model treats cognition, memory, execution, verification, maintenance, or security as equal top-level planes, it is descriptive only, not canonical.
5. If a model describes synchronous pipeline authority instead of event-driven coordination, it is descriptive only, not canonical.
6. If a conflict exists, CONTROL PLANE canon overrides legacy layer or capability language.

---

## 6. Authority Rules

### RUNTIME

May:
- execute work
- consume authoritative control-plane state
- emit runtime results

May not:
- invent policy
- self-authorize rollouts

### DATA COLLECTION

May:
- record evidence
- generate labels

May not:
- influence runtime directly

### MONITORING

May:
- observe
- measure
- alert
- recommend rollback

May not:
- change authority directly

### LEARNING

May:
- recommend
- score
- calibrate
- evaluate

May not:
- enforce
- promote itself
- change thresholds

### CONTROL PLANE

May:
- enforce
- approve
- deny
- roll forward
- roll back
- define allowlists
- write system behavior

---

## 7. Event Bus Rules

All plane coordination is event-driven.

Canonical event families:
- `routing.events`
- `sweep.events`
- `metrics.events`
- `model.events`
- `control.events`

Canonical emission pattern:
- RUNTIME emits routing and execution events
- DATA COLLECTION emits sweep and label events
- MONITORING emits health and drift events
- LEARNING emits model and evaluation events
- CONTROL PLANE emits authoritative rollout and policy events

No plane may treat advisory events as control authority.

---

## 8. Non-Canonical Patterns To Avoid

Do not:
- split write authority across control, verification, maintenance, and security
- define cognition as a sovereign plane
- define execution as a sovereign plane
- define memory as a sovereign plane
- omit DATA COLLECTION
- omit MONITORING
- let LEARNING enforce runtime behavior
- replace event coordination with synchronous authority chaining

---

## 9. Final Mapping Table

| Concern | Canonical Plane | Legacy Alias | Canonical Status |
|---|---|---|---|
| Request entry | RUNTIME | External Interfaces / Ingress | canonical |
| Agent personas | RUNTIME | Agent Surface | canonical |
| Reasoning | RUNTIME | Cognition | subordinate view |
| Provider routing | RUNTIME + LEARNING + CONTROL PLANE | Cognition / Collective Intelligence | canonical with split concerns |
| Skill routing | RUNTIME + LEARNING + CONTROL PLANE | not formally represented in old models | canonical |
| Comparative sweeps | DATA COLLECTION | missing in old models | canonical |
| Health / latency / drift | MONITORING | missing in old models | canonical |
| Model training | LEARNING | Collective Intelligence | canonical |
| Policy writes | CONTROL PLANE | Control + Security + Verification | canonical only in CONTROL PLANE |
| Infrastructure | substrate | Data / Infrastructure / Memory | subordinate substrate |

---

## 10. Final Decision Statement

SWARM uses a **5-plane canonical architecture**.

All other models are interpreted as **secondary views**:
- implementation anatomy
- capability vocabulary
- historical context

No secondary view may redefine authority, omit required planes, or compete with canon.

---

## 11. Canonical Operating Principles

The 5-plane canon defines architectural authority. The live fleet must also follow these operating principles:

- SWARM is a hybrid cloud and on-prem orchestration, inference, and execution fabric.
- Nodes belong to SWARM by role, routing participation, private-mesh participation, and health contract rather than by hardware class alone.
- SWARM operates private-mesh-first. Internal IP networking, WireGuard, and other private control paths are the default for service-to-service communication, administration, and runtime coordination.
- Public exposure is restricted to designated edge systems and must be explicitly justified.
- SWARM is role-specialized. Control, data, worker, GPU, utility, and edge responsibilities should remain clearly separated wherever practical.
- Lane-based execution is encouraged so workloads can be routed by cost, latency, risk, and hardware fit rather than treated as one undifferentiated compute pool.
- Long-running production workloads must run under an explicit supervision contract with restart behavior, boot persistence, structured logs, and health verification.
- Singleton control nodes must remain lean, observable, and protected from storage drift or accidental workload sprawl.

These principles are canonical. Specific node names, lane names, and public-edge exceptions are not.

---

## 12. Current Operating Model

This section documents the current operating model as of `2026-04-06`. It is descriptive and operationally authoritative for the present fleet, but it is not permanent canon.

### 12.1 Current Cloud Control And Data Layer

- `sw-mainframe-01`
- `sw-data-01`

### 12.2 Current Cloud Execution Layer

- `sw-worker-01`
- `sw-worker-02`
- `sw-worker-03`

### 12.3 Current Cloud GPU Layer

- `sw-gpu-core-01`
- `sw-gpu-code-01`
- `sw-gpu-code-02`
- `sw-gpu-embed-01`
- `sw-gpu-mee-01`
- `sw-gpu-mee-02`
- `sw-gpu-voice-01`
- `sw-gpu-voice-02`
- `sw-gpu-a100-01`

### 12.4 Current On-Prem Utility And Control Layer

- `sw-ctrl-01`
- `sw-util-01`
- `sw-util-02`
- `sw-util-03`
- `sw-util-04`

### 12.5 Current Edge And Application Layer

- `portal-ai-01`
- `calculus-web-01`
- `forge-app-01`
- `forge-app-02`

### 12.6 Current Lane Names

Current lane names are part of the operating model, not permanent canon. Examples include:

- `coding-gpu`
- `voice-gpu`
- `embed-gpu`
- `mee-gpu`
- `staging-gpu`
- `a100-gpu`
- `cheap,batch`
- `verification,compliance_strict`

Lane names may evolve as workloads, hardware, and routing policy change.

---

## 13. Active Exceptions

This section records temporary exceptions to the preferred operating model. These exceptions are dated and should be retired when their dependency is removed.

- `sw-mainframe-01` currently retains a public IP as a temporary operational bridge while internal-only administration is being finalized.
- Any surviving direct-access patterns that bypass the preferred private-mesh-first model are transitional and should be retired.
- Any long-running workloads still using ad hoc background execution instead of supervised services are temporary exceptions and should be promoted into the supervision model.
- Current lane names, lane placements, and exact public-edge assignments reflect the present operating model and are not eternal truths of the canon.

All active exceptions should be documented with enough context to explain why they still exist and what condition will retire them.
