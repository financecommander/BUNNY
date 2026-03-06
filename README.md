<p align="center">
  <img src="https://img.shields.io/badge/BUNNY-AI%20Machine%20Defender-ff4500?style=for-the-badge&logo=rust&logoColor=white" alt="BUNNY Badge"/>
  <img src="https://img.shields.io/badge/Triton%20Powered-2--bit%20Ternary%20AI-blueviolet?style=for-the-badge" alt="Triton Badge"/>
  <img src="https://img.shields.io/badge/License-AGPL--3.0--or--later-green?style=for-the-badge" alt="License"/>
</p>

<h1 align="center">🐰 BUNNY — AI Machine Defender</h1>

<p align="center">
  <strong>On-device AI security for every IP on your network.</strong><br/>
  Rust core · Flutter cross-platform UI · Triton ternary neural inference<br/>
  Protects and safeguards the <a href="https://github.com/financecommander/super-duper-spork">super-duper-spork</a> Swarm mainframe.
</p>

---

## What is BUNNY?

BUNNY is an **AI-powered, edge-first security product** that defends every device on your network — routers, smart TVs, Raspberry Pis, phones, laptops, and IoT endpoints. It runs a fleet of autonomous security agents, each powered by ultra-efficient **2-bit ternary neural networks** compiled through [Triton](https://github.com/financecommander/Triton), delivering 4–16× model compression and 2–3× inference speedup compared to traditional FP32 models.

BUNNY is the **on-device/edge security layer** of the financecommander ecosystem. It works in tight symbiosis with the central **[super-duper-spork](https://github.com/financecommander/super-duper-spork)** Swarm mainframe — BUNNY protects the infrastructure that super-duper-spork orchestrates.

---

## Architecture

````text
┌─────────────────────────────────────────────────────────────┐
│                    super-duper-spork                         │
│              (Swarm Mainframe / Orchestrator)                │
│         SendGrid · GHL · LLM pipelines · Agents             │
└──────────────────────┬──────────────────────────────────────┘
                       │  gRPC / mTLS
┌──────────────────────▼──────────────────────────────────────┐
│                        🐰 BUNNY                             │
│  ┌──────────┐ ┌──────────┐ ┌─────────┐ ┌────────────────┐  │
│  │  Threat  │ │    AI    │ │ Sandbox │ │  IoT Firewall  │  │
│  │  Hunter  │ │ Guardian │ │  Agent  │ │     Agent      │  │
│  └────┬─────┘ └────┬─────┘ └────┬────┘ └───────┬────────┘  │
│       │            │            │               │           │
│       └────────────┴────────────┴───────────────┘           │
│                    Triton Ternary Engine                     │
│               (2-bit models · ./triton-core)                │
│  ┌──────────────────────────────────────────────────────┐   │
│  │               bunny-core (Rust)                      │   │
│  │  Licensing · Turso DB · LanceDB vectors · Stripe     │   │
│  └──────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────┐   │
│  │            Flutter UI (cross-platform)                │   │
│  │   Dashboard · Device Manager · Upgrade Flow           │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
````

---

## Security Agents

| Agent | Role | Triton Model |
|-------|------|-------------|
| **🔍 Threat Hunter** | Polls live threat feeds (Abuse.ch, OTX), correlates IOCs against local network traffic via vector similarity search (LanceDB). | Ternary anomaly classifier |
| **🛡️ AI Guardian** | Monitors all traffic flowing to/from super-duper-spork (SendGrid webhooks, GHL API calls, LLM inference requests). Detects prompt injection, data exfiltration, and API abuse. | Ternary traffic sentinel |
| **💣 Sandbox Agent** | Detonates suspicious payloads in Firecracker microVMs. Verdicts returned with confidence scores. | Ternary malware classifier |
| **📡 IoT Firewall** | Deep packet inspection at the edge for IoT/smart devices. Blocks C2 callbacks, DNS tunneling, and lateral movement. | Ternary packet filter |
| **🧠 Learning & Adaptation** | Continuously retrains local ternary models from new threat data without cloud dependency. Federated learning ready. | Triton on-device trainer |

---

## Project Structure

````text
BUNNY/
├── Cargo.toml                 # Rust workspace root
├── .env.example               # Environment variables template
├── triton-core/               # Triton ternary engine (git submodule)
├── crates/
│   ├── core/                  # bunny-core: licensing, DB, vectors, gRPC, auth
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   └── licensing.rs   # First-IP-free + $1/mo per extra IP (Stripe)
│   │   └── build.rs           # Protobuf / gRPC codegen (tonic-build)
│   ├── agents/                # bunny-agents: all 5 security agent personas
│   │   └── src/
│   ├── network/               # bunny-network: packet capture, DPI, firewall rules
│   │   └── src/
│   └── sandbox/               # bunny-sandbox: Firecracker microVM detonation
│       └── src/
│           ├── lib.rs
│           ├── firecracker.rs # MicroVM payload detonation
│           └── threat_hunter.rs
├── flutter/
│   ├── pubspec.yaml           # Flutter app (v0.6.0+6)
│   └── lib/
│       └── main.dart          # Cross-platform dashboard UI
└── README.md
````

---

## Triton Integration

BUNNY's brain runs exclusively on **Triton** — the ternary neural network DSL from [`financecommander/Triton`](https://github.com/financecommander/Triton).

- **All five agents** use Triton-compiled 2-bit ternary models for inference
- Models are stored in `./triton-core` (git submodule) or loaded via `cargo` dependency
- **No FP32 models or heavy frameworks** — optimized for constrained hardware:
  - 📱 Mobile (iOS / Android)
  - 📡 Routers & gateways
  - 📺 Smart TVs
  - 🍓 Raspberry Pi & embedded Linux

```rust
// Example: Loading a Triton ternary model in an agent
use triton_core::TernaryModel;

let model = TernaryModel::load("./triton-core/models/threat_classifier.tern")?;
let verdict = model.infer(&packet_features)?; // 2-bit ternary inference
```

---

## Monetization Model

BUNNY uses a **unique per-IP licensing model** — code lives exclusively in this repo.

| Tier | Price | Details |
|------|-------|---------|
| **Free** | $0 forever | First detected IP/device is always protected free |
| **Pro** | $1/month per IP | Each additional IP the user selects via dashboard |

Users choose which IPs to protect through the Flutter dashboard. Billing is handled via **Stripe** (subscriptions auto-created in `bunny-core` licensing module).

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| **Core engine** | Rust 1.85+ (edition 2024) |
| **AI inference** | Triton ternary DSL (2-bit quantized models) |
| **Cross-platform UI** | Flutter 3.29+ via `flutter_rust_bridge` FFI |
| **Database** | Turso (libSQL) — edge-replicated SQLite |
| **Vector store** | LanceDB — local-first vector similarity |
| **Sandbox** | Firecracker microVMs |
| **Billing** | Stripe (subscriptions + webhooks) |
| **Auth** | JWT (`jsonwebtoken` with AWS LC RS) |
| **RPC** | gRPC via Tonic + Prost (protobuf) |
| **Ecosystem link** | super-duper-spork (Swarm mainframe) |

---

## Quick Start

### Prerequisites

- Rust ≥ 1.85 (edition 2024)
- Flutter ≥ 3.29
- Firecracker (optional, for sandbox agent)

### Setup

```bash
# Clone with Triton submodule
git clone --recurse-submodules https://github.com/financecommander/BUNNY.git
cd BUNNY

# Configure environment
cp .env.example .env
# Fill in: TURSO_URL, TURSO_AUTH_TOKEN, JWT_SECRET, STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET

# Build Rust workspace
cargo build --release

# Run Flutter dashboard
cd flutter
flutter pub get
flutter run
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `TURSO_URL` | Turso database URL (`libsql://your-db.turso.io`) |
| `TURSO_AUTH_TOKEN` | Turso authentication token |
| `JWT_SECRET` | Secret key for JWT signing |
| `STRIPE_SECRET_KEY` | Stripe secret key for billing |
| `STRIPE_WEBHOOK_SECRET` | Stripe webhook signing secret |

---

## Ecosystem Map

````text
financecommander/
├── BUNNY                  ← You are here (AI Machine Defender)
├── Triton                 ← Ternary neural network DSL (BUNNY's brain)
└── super-duper-spork      ← Swarm mainframe (BUNNY protects this)
````

---

## Roadmap

- [x] Phase 6 — Cross-platform Flutter dashboard
- [x] Rust workspace with core, agents, network, sandbox crates
- [x] Licensing engine (first-IP-free + Stripe billing)
- [x] Firecracker sandbox detonation
- [x] Threat Hunter with live feed polling
- [ ] Full Triton ternary model integration across all agents
- [ ] AI Guardian — super-duper-spork traffic monitoring
- [ ] IoT Firewall — edge DPI with ternary packet classification
- [ ] Learning & Adaptation — on-device federated retraining
- [ ] Rust ↔ Flutter FFI via `flutter_rust_bridge`
- [ ] Production Stripe subscription flows
- [ ] Multi-platform release (macOS, Windows, Linux, Android, iOS)

---

## Shapeshifter Architecture Goals

### Adaptive AI Execution, Compression, and Validation System

This section outlines the **future build-out plan** for the Shapeshifter architecture across the existing repository ecosystem.

Shapeshifter introduces:

- adaptive model routing
- compression-aware execution
- layered validation
- distributed worker execution
- telemetry-driven optimization

The goal is to evolve the current centralized swarm architecture into a **task-adaptive AI infrastructure platform**.

#### System Overview

Shapeshifter enables the system to dynamically adjust:

- model size
- compression level
- workflow topology
- compute location
- validation intensity

based on task classification and operational telemetry.

Execution pipeline:

```

task input
↓
task classification
↓
compression profile selection
↓
workflow template selection
↓
execution
↓
validation
↓
escalation (if required)

```

#### Repository Architecture

Shapeshifter builds on the current repository structure.

| Repository | Role |
|---|---|
| `super-duper-spork` | control plane orchestration |
| `Orchestra` | workflow DSL |
| `Triton` | model compilation and compression |
| `AI-PORTAL` | evaluation and experiment management |
| `ProbFlow` | probabilistic routing and uncertainty |
| `BUNNY` | edge worker runtime |

#### Core Architectural Layers

##### Control Plane

Repository: `super-duper-spork`

Responsibilities:

- task intake and routing
- workflow orchestration
- escalation control
- validation ladder management
- telemetry aggregation

Future additions:

```

/routing
/task_classifier
/validation_ladder
/compression_router
/worker_registry

```

##### Workflow Definition

Repository: `Orchestra`

Responsibilities:

- workflow templates
- swarm execution topology
- recursive task decomposition
- validation pipeline definitions

Future additions:

```

/workflow_templates
/shapes
fast_path
reviewer_path
swarm_path
hierarchical_path

```

##### Model Runtime and Compression

Repository: `Triton`

Responsibilities:

- model compilation
- compression pipelines
- ternary runtime kernels
- mixed precision export
- hardware-target optimization

Future additions:

```

/compression_profiles
planner_safe
specialist_balanced
worker_fast
edge_extreme

/layer_sensitivity
/runtime_metrics
/export_targets

```

##### Model Lifecycle and Evaluation

Repository: `AI-PORTAL`

Responsibilities:

- model registry
- experiment tracking
- compression benchmarking
- dataset management
- telemetry dashboards

Future additions:

```

/models
/experiments
/compression_benchmarks
/task_family_metrics

```

##### Routing Intelligence

Repository: `ProbFlow`

Responsibilities:

- uncertainty scoring
- routing optimization
- compression profile selection
- escalation thresholds

Future additions:

```

/routing_models
/confidence_scoring
/expected_value_estimation

```

##### Edge Worker Runtime

Repository: `BUNNY`

Responsibilities:

- lightweight worker runtime
- constrained-device execution
- secure remote worker protocol
- compressed model execution

Future additions:

```

/worker_runtime
/task_executor
/telemetry_client

```

#### Compression Strategy

Compression must be treated as a **routing primitive** rather than a static model property.

The system uses compression profiles.

##### Profiles

###### Planner Safe

Purpose:

- architecture planning
- complex reasoning
- arbitration

Compression:

- minimal quantization
- mixed precision

###### Specialist Balanced

Purpose:

- domain reasoning
- medium complexity tasks

Compression:

- 4–5 bit quantization
- mixed precision

###### Worker Fast

Purpose:

- bounded execution tasks
- high-throughput workers

Compression:

- aggressive quantization
- optional ternary

###### Edge Extreme

Purpose:

- edge nodes
- constrained environments

Compression:

- ternary models
- ultra-low memory footprint

#### Ternary Compression Policy

Ternary compression is applied selectively.

Use cases:

- swarm workers
- microtasks
- edge nodes

Avoid ternary for:

- planners
- reviewer models
- complex reasoning tasks

Layer sensitivity rules:

| Layer | Compression Policy |
|---|---|
| Embeddings | preserve |
| Attention projections | moderate compression |
| Feed-forward layers | aggressive compression |
| Output head | preserve |

#### Task Classification

Tasks are categorized before execution.

| Class | Example Tasks |
|---|---|
| Compress | formatting, tests, lint fixes |
| Balance | bug fixes, repo review |
| Preserve | architecture changes |

Classification inputs:

- scope
- risk
- context size
- expected reasoning depth

#### Validation Architecture

Validation uses a multi-stage ladder.

##### Level 1 — Deterministic Checks

Examples:

- syntax validation
- lint checks
- type checking
- schema validation

##### Level 2 — Semantic Checks

Examples:

- unit tests
- static analysis
- policy rules

##### Level 3 — Reviewer Models

Examples:

- code reviewer
- security reviewer
- compliance reviewer

##### Level 4 — Human Review

Required for:

- high-risk outputs
- unresolved conflicts
- regulatory workflows

#### Worker Self-Validation

Workers return validation metadata.

Example:

```json
{
  "task_id": "123",
  "confidence": 0.84,
  "checks": {
    "lint_pass": true,
    "tests_passed": true
  }
}
```

This allows early rejection of low-quality outputs.

#### Telemetry System

Telemetry is required for adaptive routing.

Metrics tracked:

* success rate
* validation pass rate
* latency
* compression profile performance
* escalation frequency

Repositories responsible:

```
Triton → runtime metrics
AI-PORTAL → dashboards
super-duper-spork → routing telemetry
```

#### Failure Escalation

Escalation ladder:

```
Compress → Balance → Preserve
```

Triggers:

* validation failure
* low confidence
* repeated retries

#### Execution Examples

##### Small Task

```
classification → compress
model → worker_fast
validation → deterministic checks
```

##### Medium Task

```
classification → balance
workflow → planner + workers
validation → semantic checks + reviewer
```

##### Large Task

```
classification → preserve
workflow → hierarchical swarm
validation → reviewer chain + human approval
```

#### Implementation Roadmap

##### Phase 1 — Validation Infrastructure

Repositories:

```
super-duper-spork
Orchestra
```

Deliverables:

* validation ladder
* worker self-check protocol
* task classification system

##### Phase 2 — Compression Profiles

Repositories:

```
Triton
AI-PORTAL
```

Deliverables:

* compression profiles
* benchmark dashboards
* mixed precision policies

##### Phase 3 — Adaptive Routing

Repositories:

```
super-duper-spork
ProbFlow
```

Deliverables:

* compression-aware routing
* confidence scoring
* escalation policies

##### Phase 4 — Distributed Worker Expansion

Repositories:

```
BUNNY
super-duper-spork
```

Deliverables:

* worker registry
* remote task execution
* edge worker support

##### Phase 5 — Telemetry Optimization

Repositories:

```
AI-PORTAL
Triton
ProbFlow
```

Deliverables:

* compression telemetry
* routing optimization
* performance analytics

#### Engineering Principles

Shapeshifter follows a single rule:

> Use the smallest reliable model and workflow capable of completing the task safely.

Compression, routing, and validation all support this objective.

#### Expected Outcomes

Compared to traditional single-model systems:

* improved compute efficiency
* reduced validation burden
* higher scalability
* adaptive model deployment
* improved failure containment

#### Long-Term Vision

Shapeshifter evolves the existing architecture into:

```
Adaptive AI Infrastructure Platform
```

Where:

* routing decisions are telemetry-driven
* models are dynamically selected
* workflows adapt to task complexity
* validation is automated by default


## Shapeshifter Architecture Goals

### Adaptive AI Execution, Compression, and Validation System

This section outlines the **future build-out plan** for the Shapeshifter architecture across the existing repository ecosystem.

Shapeshifter introduces:

- adaptive model routing
- compression-aware execution
- layered validation
- distributed worker execution
- telemetry-driven optimization

The goal is to evolve the current centralized swarm architecture into a **task-adaptive AI infrastructure platform**.

#### System Overview

Shapeshifter enables the system to dynamically adjust:

- model size
- compression level
- workflow topology
- compute location
- validation intensity

based on task classification and operational telemetry.

Execution pipeline:

```

task input
↓
task classification
↓
compression profile selection
↓
workflow template selection
↓
execution
↓
validation
↓
escalation (if required)

```

#### Repository Architecture

Shapeshifter builds on the current repository structure.

| Repository | Role |
|---|---|
| `super-duper-spork` | control plane orchestration |
| `Orchestra` | workflow DSL |
| `Triton` | model compilation and compression |
| `AI-PORTAL` | evaluation and experiment management |
| `ProbFlow` | probabilistic routing and uncertainty |
| `BUNNY` | edge worker runtime |

#### Core Architectural Layers

##### Control Plane

Repository: `super-duper-spork`

Responsibilities:

- task intake and routing
- workflow orchestration
- escalation control
- validation ladder management
- telemetry aggregation

Future additions:

```

/routing
/task_classifier
/validation_ladder
/compression_router
/worker_registry

```

##### Workflow Definition

Repository: `Orchestra`

Responsibilities:

- workflow templates
- swarm execution topology
- recursive task decomposition
- validation pipeline definitions

Future additions:

```

/workflow_templates
/shapes
fast_path
reviewer_path
swarm_path
hierarchical_path

```

##### Model Runtime and Compression

Repository: `Triton`

Responsibilities:

- model compilation
- compression pipelines
- ternary runtime kernels
- mixed precision export
- hardware-target optimization

Future additions:

```

/compression_profiles
planner_safe
specialist_balanced
worker_fast
edge_extreme

/layer_sensitivity
/runtime_metrics
/export_targets

```

##### Model Lifecycle and Evaluation

Repository: `AI-PORTAL`

Responsibilities:

- model registry
- experiment tracking
- compression benchmarking
- dataset management
- telemetry dashboards

Future additions:

```

/models
/experiments
/compression_benchmarks
/task_family_metrics

```

##### Routing Intelligence

Repository: `ProbFlow`

Responsibilities:

- uncertainty scoring
- routing optimization
- compression profile selection
- escalation thresholds

Future additions:

```

/routing_models
/confidence_scoring
/expected_value_estimation

```

##### Edge Worker Runtime

Repository: `BUNNY`

Responsibilities:

- lightweight worker runtime
- constrained-device execution
- secure remote worker protocol
- compressed model execution

Future additions:

```

/worker_runtime
/task_executor
/telemetry_client

```

#### Compression Strategy

Compression must be treated as a **routing primitive** rather than a static model property.

The system uses compression profiles.

##### Profiles

###### Planner Safe

Purpose:

- architecture planning
- complex reasoning
- arbitration

Compression:

- minimal quantization
- mixed precision

###### Specialist Balanced

Purpose:

- domain reasoning
- medium complexity tasks

Compression:

- 4–5 bit quantization
- mixed precision

###### Worker Fast

Purpose:

- bounded execution tasks
- high-throughput workers

Compression:

- aggressive quantization
- optional ternary

###### Edge Extreme

Purpose:

- edge nodes
- constrained environments

Compression:

- ternary models
- ultra-low memory footprint

#### Ternary Compression Policy

Ternary compression is applied selectively.

Use cases:

- swarm workers
- microtasks
- edge nodes

Avoid ternary for:

- planners
- reviewer models
- complex reasoning tasks

Layer sensitivity rules:

| Layer | Compression Policy |
|---|---|
| Embeddings | preserve |
| Attention projections | moderate compression |
| Feed-forward layers | aggressive compression |
| Output head | preserve |

#### Task Classification

Tasks are categorized before execution.

| Class | Example Tasks |
|---|---|
| Compress | formatting, tests, lint fixes |
| Balance | bug fixes, repo review |
| Preserve | architecture changes |

Classification inputs:

- scope
- risk
- context size
- expected reasoning depth

#### Validation Architecture

Validation uses a multi-stage ladder.

##### Level 1 — Deterministic Checks

Examples:

- syntax validation
- lint checks
- type checking
- schema validation

##### Level 2 — Semantic Checks

Examples:

- unit tests
- static analysis
- policy rules

##### Level 3 — Reviewer Models

Examples:

- code reviewer
- security reviewer
- compliance reviewer

##### Level 4 — Human Review

Required for:

- high-risk outputs
- unresolved conflicts
- regulatory workflows

#### Worker Self-Validation

Workers return validation metadata.

Example:

```json
{
  "task_id": "123",
  "confidence": 0.84,
  "checks": {
    "lint_pass": true,
    "tests_passed": true
  }
}
```

This allows early rejection of low-quality outputs.

#### Telemetry System

Telemetry is required for adaptive routing.

Metrics tracked:

* success rate
* validation pass rate
* latency
* compression profile performance
* escalation frequency

Repositories responsible:

```
Triton → runtime metrics
AI-PORTAL → dashboards
super-duper-spork → routing telemetry
```

#### Failure Escalation

Escalation ladder:

```
Compress → Balance → Preserve
```

Triggers:

* validation failure
* low confidence
* repeated retries

#### Execution Examples

##### Small Task

```
classification → compress
model → worker_fast
validation → deterministic checks
```

##### Medium Task

```
classification → balance
workflow → planner + workers
validation → semantic checks + reviewer
```

##### Large Task

```
classification → preserve
workflow → hierarchical swarm
validation → reviewer chain + human approval
```

#### Implementation Roadmap

##### Phase 1 — Validation Infrastructure

Repositories:

```
super-duper-spork
Orchestra
```

Deliverables:

* validation ladder
* worker self-check protocol
* task classification system

##### Phase 2 — Compression Profiles

Repositories:

```
Triton
AI-PORTAL
```

Deliverables:

* compression profiles
* benchmark dashboards
* mixed precision policies

##### Phase 3 — Adaptive Routing

Repositories:

```
super-duper-spork
ProbFlow
```

Deliverables:

* compression-aware routing
* confidence scoring
* escalation policies

##### Phase 4 — Distributed Worker Expansion

Repositories:

```
BUNNY
super-duper-spork
```

Deliverables:

* worker registry
* remote task execution
* edge worker support

##### Phase 5 — Telemetry Optimization

Repositories:

```
AI-PORTAL
Triton
ProbFlow
```

Deliverables:

* compression telemetry
* routing optimization
* performance analytics

#### Engineering Principles

Shapeshifter follows a single rule:

> Use the smallest reliable model and workflow capable of completing the task safely.

Compression, routing, and validation all support this objective.

#### Expected Outcomes

Compared to traditional single-model systems:

* improved compute efficiency
* reduced validation burden
* higher scalability
* adaptive model deployment
* improved failure containment

#### Long-Term Vision

Shapeshifter evolves the existing architecture into:

```
Adaptive AI Infrastructure Platform
```

Where:

* routing decisions are telemetry-driven
* models are dynamically selected
* workflows adapt to task complexity
* validation is automated by default

## License

**AGPL-3.0-or-later** — See [LICENSE](LICENSE) for details.

---

<p align="center">
  <strong>🐰 BUNNY sees everything. BUNNY protects everything.</strong><br/>
  <em>Powered by Triton ternary AI · Guarding the super-duper-spork Swarm</em>
</p>
