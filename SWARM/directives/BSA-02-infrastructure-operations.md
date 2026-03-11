# BUNNY-SWARM INFRASTRUCTURE & OPERATIONS DIRECTIVE

**Directive ID:** BSA-02
**System:** Calculus AI Platform
**Authority:** BUNNY Core + BUNNY Shield
**Classification:** Internal System Deployment Directive

---

## 1. Purpose

This directive establishes the production infrastructure, orchestration, and operational requirements for the Calculus AI system.

The directive defines:
- Infrastructure topology
- Orchestration framework
- Model routing requirements
- Memory architecture
- Security enforcement
- Telemetry and observability
- Deployment priorities

The goal is to transform from multi-assistant tooling into a distributed autonomous intelligence system.

## 2. Core Operating Principle

The system must operate as a distributed compute organism, not a collection of independent assistants.

All agent activity must flow through the following control chain:

```
Security Plane → Control Plane → Cognition Plane → Execution Plane → Memory Plane → Collective Intelligence Plane
```

No agent may bypass this flow.

## 3. Infrastructure Topology

The system must deploy across four primary infrastructure domains.

### 3.1 Control Infrastructure

**Purpose:** Coordinate system activity.

**Responsibilities:**
- Orchestrator runtime
- Job scheduling
- Agent lifecycle management
- Task decomposition
- Execution governance

**Recommended Technologies:** Ray, Temporal, LangGraph, Prefect

**Deployment Modules:**
```
/opt/swarm/orchestrator/
/opt/swarm/scheduler/
/opt/swarm/planner/
/opt/swarm/governors/
```

### 3.2 Model Compute Infrastructure

**Purpose:** Execute reasoning and inference workloads.

**Compute Classes:**

| Class | Hardware | Purpose |
|---|---|---|
| Fast inference | NVIDIA L4 GPU | Ternary model inference |
| Reasoning | Cloud APIs (OpenAI, Anthropic, Grok) | Complex reasoning |
| Classifier | Small local models | Ternary routing |

**Routing Policy:**
- Smallest capable model first
- Escalate only when required

### 3.3 Memory Infrastructure

**Purpose:** Persistent system awareness.

**Required Components:**
- Vector memory
- Event logging
- Knowledge graph
- Context storage
- Agent state registry

**Recommended Technologies:**
| Component | Technology |
|---|---|
| Vector memory | Qdrant / Weaviate |
| State + logs | PostgreSQL |
| Knowledge graph | Neo4j |
| Context cache | Redis |

**Memory Modules:**
```
/opt/swarm/memory/
/opt/swarm/vector_store/
/opt/swarm/event_log/
/opt/swarm/knowledge_graph/
/opt/swarm/context_cache/
```

All actions must be logged as atomic event objects.

### 3.4 Messaging Infrastructure

**Purpose:** Enable inter-agent communication.

**Required Messaging Layer:** Kafka, NATS, Redis Streams

**Functions:**
- Agent event transport
- Task queueing
- Pipeline coordination
- Agent messaging

**Messaging Modules:**
```
/opt/swarm/bus/
/opt/swarm/events/
/opt/swarm/tasks/
```

## 4. Agent Structure

Agents must be classified into four types.

### 4.1 Executive Agent

**Agent:** BUNNY CORE

**Responsibilities:**
- Directive intake
- Strategy formation
- Task decomposition
- System coordination

### 4.2 Security Agent

**Agent:** BUNNY SHIELD

**Responsibilities:**
- Identity verification
- Tool permission enforcement
- Crypto operations
- Audit logging
- System anomaly detection

### 4.3 Dispatcher Agents

**Example:** Jack Dispatcher

**Responsibilities:**
- Task classification
- Intent routing
- Model selection
- Workflow routing

Dispatcher models should use small fast classifiers, not full reasoning models.

### 4.4 Worker Agents

**Responsibilities:**
- Code generation
- Automation tasks
- Data ingestion
- Analysis jobs
- External tool interaction

Worker agents operate inside the execution plane.

## 5. Voice System Architecture

Voice systems must follow this pipeline:

```
Speech Input
    ↓
Speech-to-Text (Whisper / Deepgram)
    ↓
Intent Classifier (Ternary classifier)
    ↓
Model Router
    ↓
Reasoning Model (GPT-5.4 / Claude)
    ↓
Response Generation
    ↓
Text-to-Speech (Cartesia / ElevenLabs)
```

VoiceAI must operate as a skill within the interface plane, not a standalone reasoning system.

## 6. Memory Policy

Every execution must generate an event record.

**Event Structure:**
| Field | Description |
|---|---|
| Event_ID | Unique identifier |
| Timestamp | ISO 8601 |
| Agent_ID | Executing agent |
| Action_Type | Classification |
| Input_Context | Input data |
| Execution_Result | Output data |
| Confidence_Score | 0.0 - 1.0 |

Event records must feed:
- Memory store
- Knowledge graph
- Meta-learning engine

This enables temporal reasoning and process reconstruction.

## 7. Security Enforcement

All operations must pass through BUNNY Shield validation.

**Security Functions:**
- Agent authentication
- Tool access authorization
- API key protection
- Crypto key management
- Execution sandboxing
- Audit logging

**Security Modules:**
```
/opt/swarm/security/
/opt/swarm/identity/
/opt/swarm/crypto/
/opt/swarm/audit/
```

**Security Policies Must Enforce:**
- Least privilege access
- Agent role boundaries
- Tool permission controls

## 8. Observability Requirements

The system must maintain continuous telemetry.

**Required Monitoring Metrics:**
- Agent latency
- Task success rate
- Model token usage
- Routing accuracy
- Error frequency
- System health

**Recommended Monitoring Stack:** OpenTelemetry, Prometheus, Grafana, Langfuse

**Observability Modules:**
```
/opt/swarm/telemetry/
/opt/swarm/monitoring/
/opt/swarm/tracing/
/opt/swarm/benchmarks/
```

## 9. Deployment Phases

| Phase | Domain | Components |
|---|---|---|
| 1 | Security Plane | Identity system, Key management |
| 2 | Control Plane | Orchestrator, Dispatcher agents |
| 3 | Cognition Plane | Model routing, Policy engines |
| 4 | Execution Plane | Workers, Automation pipelines |
| 5 | Memory Plane | Vector store, Knowledge graph, Event logs |
| 6 | Interface Plane | APIs, Skills, Connectors |
| 7 | Observability | Monitoring, Diagnostics |
| 8 | Hivemind | Meta-learning, Collective optimization |

## 10. Performance Principles

The system must prioritize:
- Low-latency routing
- Smallest capable model
- Parallel agent execution
- Persistent event logging
- Distributed memory awareness

GPU resources must be reserved primarily for:
- Ternary inference
- High-compute reasoning tasks

## 11. Governance

All modules must comply with:
- **BSA-01** — Architecture Directive
- **BSA-02** — Infrastructure & Operations Directive

Unauthorized components must not be introduced into the runtime.

## 12. Directive Activation

This directive establishes the production infrastructure blueprint for Calculus AI systems.

All development teams implementing components must conform to this structure.

BUNNY Core retains authority to update or extend directives as the system evolves.
