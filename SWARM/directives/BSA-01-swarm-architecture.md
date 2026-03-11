# BSA-01 — Swarm Architecture Directive

**System:** Calculus AI Platform
**Authority:** BUNNY Core + BUNNY Shield
**Directive Type:** System Architecture and Agent Topology
**Classification:** Internal System Architecture Directive

---

## 1. Purpose

This directive establishes the foundational architecture of the Calculus AI system.

The directive defines:
- Multi-plane system architecture
- Agent hierarchy and roles
- Communication protocols between planes
- Data flow architecture
- Model routing architecture
- API surface definition
- Deployment topology

All subsystems, agents, and infrastructure components must conform to this architecture. No component may operate outside the boundaries defined here.

## 2. Core Architecture Principle

The Calculus AI system operates as a multi-plane distributed intelligence organism.

All system activity must flow through six architectural planes in strict order:

```
Security Plane → Control Plane → Cognition Plane → Execution Plane → Memory Plane → Collective Intelligence Plane
```

No agent, service, or component may bypass this flow. Every request entering the system traverses all applicable planes before producing output.

## 3. Multi-Plane Architecture

### 3.1 Security Plane

**Purpose:** Enforce trust, identity, and access boundaries across all system activity.

**Responsibilities:**
- Agent identity verification
- Request authentication and authorization
- Tool permission enforcement
- Crypto key management
- Audit trail generation
- Anomaly and threat detection
- Data classification enforcement

**Governing Agent:** BUNNY Shield

**Deployment Modules:**
```
/opt/swarm/security/
/opt/swarm/identity/
/opt/swarm/crypto/
/opt/swarm/audit/
```

**Policy:** Every request must pass Security Plane validation before entering the Control Plane. Rejected requests are logged and terminated.

### 3.2 Control Plane

**Purpose:** Coordinate, schedule, and govern all system activity.

**Responsibilities:**
- Directive intake and interpretation
- Strategy formation
- Task decomposition
- Job scheduling and prioritization
- Agent lifecycle management
- Execution governance
- Resource allocation

**Governing Agent:** BUNNY Core

**Deployment Modules:**
```
/opt/swarm/orchestrator/
/opt/swarm/scheduler/
/opt/swarm/planner/
/opt/swarm/governors/
```

**Policy:** The Control Plane decomposes high-level directives into executable task graphs. Task assignment follows agent capability matching and resource availability.

### 3.3 Cognition Plane

**Purpose:** Execute reasoning, classification, and decision-making workloads.

**Responsibilities:**
- Intent classification
- Model selection and routing
- Reasoning chain execution
- Policy evaluation
- Knowledge retrieval integration
- Response synthesis

**Compute Classes:**

| Class | Hardware | Purpose |
|---|---|---|
| Fast Inference | NVIDIA L4 GPU / Local models | Ternary classification, lightweight inference |
| Reasoning | Cloud APIs (OpenAI, Anthropic, Grok) | Complex reasoning, multi-step analysis |
| Classifier | Small local models | Intent routing, task categorization |
| Embedding | GPU / CPU | Vector embedding generation |

**Deployment Modules:**
```
/opt/swarm/cognition/
/opt/swarm/models/
/opt/swarm/routing/
/opt/swarm/policy_engine/
```

### 3.4 Execution Plane

**Purpose:** Perform concrete work including code generation, data processing, automation, and external tool interaction.

**Responsibilities:**
- Code generation and execution
- Data ingestion and transformation
- API interaction
- Automation pipeline execution
- File and system operations
- External service integration

**Governing Agents:** Worker Agents (supervised by BUNNY Core)

**Deployment Modules:**
```
/opt/swarm/workers/
/opt/swarm/automation/
/opt/swarm/pipelines/
/opt/swarm/tools/
```

**Policy:** Worker agents operate exclusively within the Execution Plane. All tool access must be authorized by BUNNY Shield. All outputs must be logged to the Memory Plane.

### 3.5 Memory Plane

**Purpose:** Maintain persistent system awareness, context, and event history.

**Responsibilities:**
- Vector memory storage and retrieval
- Knowledge graph maintenance
- Event log persistence
- Agent state management
- Context caching
- Temporal reasoning support

**Storage Components:**

| Component | Technology | Purpose |
|---|---|---|
| Vector Memory | Qdrant / Weaviate / pgvector | Semantic search, embedding storage |
| Knowledge Graph | Neo4j / ArangoDB | Entity relationships, structured knowledge |
| Event Log | PostgreSQL / Kafka | Execution history, audit trail |
| Context Cache | Redis | Short-term context, session state |
| Agent State | PostgreSQL | Agent configuration, lifecycle state |

**Deployment Modules:**
```
/opt/swarm/memory/
/opt/swarm/vector_store/
/opt/swarm/event_log/
/opt/swarm/knowledge_graph/
/opt/swarm/context_cache/
```

**Policy:** Every execution must produce an event record. All records must be indexed and available for retrieval by authorized agents.

### 3.6 Collective Intelligence Plane

**Purpose:** Enable system-wide learning, optimization, and emergent intelligence.

**Responsibilities:**
- Meta-learning from execution history
- Cross-agent pattern discovery
- Routing optimization
- Collective knowledge synthesis
- Performance trend analysis
- Capability gap identification

**Deployment Modules:**
```
/opt/swarm/hivemind/
/opt/swarm/meta_learning/
/opt/swarm/collective/
/opt/swarm/optimization/
```

**Policy:** The Collective Intelligence Plane operates on aggregated data from the Memory Plane. Proposed optimizations must pass validation gates defined in BSA-03 before deployment.

## 4. Agent Hierarchy

The system defines four agent tiers with strict role separation.

### 4.1 BUNNY Core — Executive Agent

**Role:** System executive. Strategic coordination and directive authority.

**Responsibilities:**
- Directive intake and interpretation
- Strategy formation and task decomposition
- System-wide coordination
- Governance enforcement
- Resource allocation decisions
- Improvement authorization

**Authority:** Highest authority in the system. All other agents operate under directives issued by BUNNY Core.

**Constraints:** BUNNY Core does not execute tasks directly. It delegates to Dispatcher and Worker agents.

### 4.2 BUNNY Shield — Security Agent

**Role:** Security enforcement across all planes.

**Responsibilities:**
- Identity verification for all agents and requests
- Tool and API access authorization
- Crypto key management and operations
- Audit logging and compliance
- Anomaly detection and threat response
- Data classification enforcement
- Experiment and deployment approval (security dimension)

**Authority:** Veto authority over any operation that violates security policy. BUNNY Shield may halt any agent or process.

**Constraints:** BUNNY Shield does not initiate tasks. It validates, authorizes, and audits.

### 4.3 Dispatcher Agents — Task Routing

**Example:** Jack Dispatcher (Calculus team)

**Role:** Classify incoming requests and route tasks to appropriate workers or models.

**Responsibilities:**
- Intent classification
- Task categorization
- Model selection
- Workflow routing
- Priority assignment

**Task Categories (Jack):**
- `gradient` — Gradient computation and optimization
- `optimizer` — Optimization algorithms
- `similarity` — Similarity and distance computation
- `statistics` — Statistical analysis
- `symbolic` — Symbolic computation
- `tensor_ops` — Tensor operations

**Authority:** Dispatcher agents assign tasks. They cannot modify code, repositories, VMs, or infrastructure.

**Model Policy:** Dispatchers must use small fast classifiers, not full reasoning models.

### 4.4 Worker Agents — Task Execution

**Role:** Execute concrete tasks assigned by Dispatchers or BUNNY Core.

**Responsibilities:**
- Code generation and modification
- Data ingestion and analysis
- Automation execution
- External tool interaction
- Report generation

**Authority:** Worker agents operate within the Execution Plane. They execute only the tasks assigned to them and report results.

**Constraints:** Workers cannot self-assign tasks, modify system configuration, or access resources outside their authorization scope.

## 5. Communication Protocols

### 5.1 Inter-Plane Communication

All communication between planes must use the system message bus.

**Messaging Infrastructure:** Kafka, NATS, or Redis Streams

**Message Format:**

| Field | Description |
|---|---|
| Message_ID | Unique identifier (UUID) |
| Timestamp | ISO 8601 |
| Source_Plane | Originating plane |
| Target_Plane | Destination plane |
| Source_Agent | Sending agent ID |
| Target_Agent | Receiving agent ID (or broadcast) |
| Message_Type | command, event, query, response |
| Payload | Structured message content |
| Priority | critical, high, normal, low |
| Correlation_ID | Links related messages |

### 5.2 Agent-to-Agent Communication

Agents communicate through the message bus. Direct agent-to-agent connections are prohibited.

**Communication Patterns:**
- **Command:** One agent directs another to perform an action
- **Event:** An agent broadcasts a state change or completion
- **Query:** An agent requests information from another agent or the Memory Plane
- **Response:** Reply to a query with requested data

### 5.3 External Communication

External communication (APIs, webhooks, third-party services) must pass through:
1. Security Plane authentication
2. Control Plane authorization
3. Execution Plane processing

No agent may communicate externally without Security Plane clearance.

**Messaging Modules:**
```
/opt/swarm/bus/
/opt/swarm/events/
/opt/swarm/tasks/
/opt/swarm/queues/
```

## 6. Data Flow Architecture

### 6.1 Request Processing Flow

```
External Request
    |
    v
Security Plane — Authenticate, authorize, classify
    |
    v
Control Plane — Decompose, schedule, assign
    |
    v
Cognition Plane — Classify intent, select model, reason
    |
    v
Execution Plane — Execute task, interact with tools
    |
    v
Memory Plane — Log event, update knowledge, cache context
    |
    v
Collective Intelligence — Analyze patterns, update optimization data
    |
    v
Response Assembly — Synthesize and return result
```

### 6.2 Event Record Structure

Every action produces an event record:

| Field | Description |
|---|---|
| Event_ID | Unique identifier |
| Timestamp | ISO 8601 |
| Agent_ID | Executing agent |
| Plane | Plane of origin |
| Action_Type | Classification of action |
| Input_Context | Input data reference |
| Execution_Result | Output data reference |
| Confidence_Score | 0.0 - 1.0 |
| Duration_MS | Execution time in milliseconds |
| Model_Used | Model identifier (if applicable) |
| Token_Count | Tokens consumed (if applicable) |

Event records feed into the Memory Plane and the Collective Intelligence Plane.

## 7. Model Routing Architecture

### 7.1 Routing Policy

The system must follow the principle of smallest capable model first.

**Routing Hierarchy:**

| Priority | Model Class | Use Case |
|---|---|---|
| 1 | Ternary Classifier | Intent classification, binary routing |
| 2 | Small Local Model | Simple generation, formatting, extraction |
| 3 | Medium Cloud Model | Multi-step reasoning, analysis |
| 4 | Large Cloud Model | Complex reasoning, strategic planning |

### 7.2 Escalation Rules

A request escalates to a larger model only when:
- The current model returns low confidence (below threshold)
- The task complexity exceeds the current model's capability
- The request explicitly requires advanced reasoning
- Previous attempt with smaller model failed quality validation

### 7.3 Model Registry

All available models must be registered in the model registry with:
- Model ID
- Provider
- Capability profile
- Cost per token
- Latency profile
- Context window
- Supported task types

### 7.4 Routing Decision Log

Every routing decision must be logged:
- Selected model
- Routing reason
- Alternative models considered
- Confidence in selection

## 8. API Surface Definition

### 8.1 Internal APIs

| API | Purpose | Consumers |
|---|---|---|
| `/api/v1/tasks` | Task submission and status | Control Plane, Dispatchers |
| `/api/v1/agents` | Agent lifecycle management | Control Plane |
| `/api/v1/memory` | Memory read/write operations | All planes |
| `/api/v1/security` | Authentication and authorization | Security Plane |
| `/api/v1/models` | Model routing and inference | Cognition Plane |
| `/api/v1/events` | Event log access | Memory Plane, Monitoring |
| `/api/v1/health` | System health and diagnostics | Monitoring |

### 8.2 External APIs

External-facing APIs are exposed under the Calculus AI branding.

| API | Purpose | Access |
|---|---|---|
| `/calculus/v1/query` | Submit queries and tasks | Authenticated clients |
| `/calculus/v1/status` | Check task status | Authenticated clients |
| `/calculus/v1/results` | Retrieve results | Authenticated clients |
| `/calculus/v1/health` | System health check | Public |

All external APIs must pass through the Security Plane. Rate limiting, authentication, and audit logging are mandatory.

## 9. Deployment Topology

### 9.1 Deployment Phases

| Phase | Domain | Components |
|---|---|---|
| 1 | Security Plane | Identity system, key management, audit logging |
| 2 | Control Plane | Orchestrator, dispatcher agents, scheduler |
| 3 | Cognition Plane | Model routing, policy engines, classifiers |
| 4 | Execution Plane | Workers, automation pipelines, tool integrations |
| 5 | Memory Plane | Vector store, knowledge graph, event logs |
| 6 | Interface Layer | External APIs, skills, connectors |
| 7 | Observability | Monitoring, diagnostics, tracing |
| 8 | Collective Intelligence | Meta-learning, collective optimization |

### 9.2 Infrastructure Requirements

| Component | Requirement |
|---|---|
| Compute | GPU instances for inference, CPU for orchestration |
| Storage | Persistent volumes for memory, graph, and event stores |
| Networking | Low-latency inter-service communication |
| Messaging | High-throughput event bus (Kafka/NATS) |
| Caching | In-memory cache layer (Redis) |
| Monitoring | Full observability stack (OpenTelemetry, Prometheus, Grafana) |

## 10. Performance Principles

The system must prioritize:
- Low-latency routing decisions
- Smallest capable model selection
- Parallel agent execution where possible
- Persistent event logging without blocking execution
- Distributed memory awareness across all planes

GPU resources must be reserved primarily for:
- Ternary inference
- Embedding generation
- High-compute reasoning tasks

## 11. Governance

All modules must comply with:
- **BSA-01** — Architecture Directive (this document)
- **BSA-02** — Infrastructure & Operations Directive
- **BSA-03** — Autonomous Self-Improvement Directive

Unauthorized components must not be introduced into the runtime.

BUNNY Core supervises architectural compliance.

BUNNY Shield enforces security compliance across all planes.

## 12. Directive Activation

This directive establishes the foundational architecture for the Calculus AI system.

All development teams, agents, and automated processes must conform to this architecture.

BUNNY Core retains authority to update or extend directives as the system evolves.
