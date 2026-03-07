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

### Phase 1 (Current) — PyTorch-backed models via Python runtime

In the current phase, Triton compiles `.tri` source files to PyTorch `nn.Module` code. BUNNY workers that have Python available can load these directly:

```python
# triton_client.py (Python path, Phase 1)
from triton_runtime import load_model

model = load_model("./triton-core/models/threat_classifier.triton")
verdict = model.infer(packet_features)  # returns {"label": "malicious", "confidence": 0.94}
```

Every Triton model artifact ships with:
- `model.pth` — packed 2-bit ternary weights
- `model_metadata.json` — capabilities, op set, hardware target, accuracy, compression ratio
- `README.md` — usage and performance notes

### Phase 2 (Next) — Pure Rust / no-Python inference

In Phase 2, Triton emits a self-contained `.triton` binary format. BUNNY's Rust workers load and execute this directly with no Python or PyTorch dependency:

```rust
// Phase 2 target interface (triton-core Rust crate)
use triton_core::runtime::TernaryRuntime;

let runtime = TernaryRuntime::load("./models/threat_classifier.triton")?;
let verdict = runtime.infer(&packet_features)?;
// returns TernaryVerdict { label: "malicious", confidence: 0.94 }
```

The `.triton` binary format contains:
1. JSON header: model name, op graph, input/output shapes, capabilities, hardware targets
2. Binary weight block: 2-bit packed ternary weights (4 trits per byte)
3. Metadata: version, compression ratio, accuracy, registered op set

**BUNNY action items for Phase 2:**
- [ ] Add `triton-core` as a proper Cargo dependency (currently submodule only)
- [ ] Implement `TernaryRuntime::load` and `infer` in `crates/core/src/triton.rs`
- [ ] Wire each agent in `crates/agents/` to use `TernaryRuntime` instead of hardcoded prompts
- [ ] Report inference latency + confidence in worker telemetry back to AI-PORTAL

---

## Phase 2 Artifact Contract

This is the **interface contract** between Triton (producer) and BUNNY (consumer). Triton owns the artifact format; BUNNY owns the loader and runtime binding.

### Artifact Directory Layout (per model)

```
models/
└── threat_classifier/
    ├── model.triton            # binary: JSON header + packed weights
    ├── model_metadata.json     # capabilities, hardware target, op set
    └── README.md               # usage, accuracy, compression stats
```

### `model_metadata.json` Schema

```json
{
  "name": "threat_classifier",
  "version": "1.0.0",
  "task": "binary_classification",
  "labels": ["benign", "malicious"],
  "input_shape": [128],
  "output_shape": [2],
  "ops": ["embedding", "linear", "relu", "classify"],
  "hardware_targets": ["cpu", "raspi", "edge"],
  "compression_ratio": 16.0,
  "accuracy": 0.94,
  "model_size_kb": 48,
  "triton_version": "0.1.0"
}
```

### Telemetry BUNNY Sends to AI-PORTAL (per inference)

```json
{
  "worker_id": "bunny-worker-abc123",
  "model": "threat_classifier",
  "inference_latency_ms": 1.2,
  "confidence": 0.94,
  "label": "malicious",
  "hardware": "raspi-4",
  "timestamp": "2026-03-07T12:00:00Z"
}
```

This telemetry drives AI-PORTAL dashboards, super-duper-spork routing updates, and ProbFlow uncertainty model refinement.

### Specialist Models BUNNY Needs (ordered by priority)

| Model | Task | Target Size | Priority |
|---|---|---|---|
| `threat_classifier` | Classify network events: benign / malicious / suspicious | < 5 MB | P0 — blocks agent wiring |
| `packet_filter` | Edge DPI: allow / block / quarantine per packet class | < 5 MB | P0 — blocks IoT Firewall |
| `malware_classifier` | Sandbox verdict: malicious / benign + confidence | < 10 MB | P1 |
| `traffic_sentinel` | Monitor API calls: normal / exfiltration / injection | < 10 MB | P1 |
| `anomaly_detector` | Time-series network anomaly detection | < 5 MB | P2 |

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
├── BUNNY                  ← You are here (AI Machine Defender / Edge Worker Runtime)
├── Triton                 ← Ternary neural network DSL + compiler + runtime (BUNNY's brain)
├── super-duper-spork      ← Swarm mainframe (BUNNY protects + executes tasks from this)
├── Orchestra              ← Workflow DSL (task graphs reference BUNNY workers as execution nodes)
├── AI-PORTAL              ← Model registry, telemetry dashboards, lifecycle management
└── ProbFlow               ← Uncertainty scoring + routing (uses BUNNY telemetry to improve routing)
````

### Cross-Repo Responsibilities (from BUNNY's perspective)

| Repo | What BUNNY provides | What BUNNY consumes |
|---|---|---|
| **Triton** | Hardware targets, edge constraints, latency/confidence telemetry | Compiled `.triton` model artifacts, op set definitions, `triton_runtime` |
| **AI-PORTAL** | Per-inference telemetry (latency, confidence, worker ID, hardware) | Model registry lookups, capability profiles, deployment instructions |
| **super-duper-spork** | Task execution results, worker availability, confidence scores | Task dispatch, model selection decisions, workflow task payloads |
| **Orchestra** | Worker status, capability declarations | Workflow graph task assignments (BUNNY workers appear as execution nodes) |
| **ProbFlow** | Confidence and latency streams per inference | Updated routing weights (which worker/model to use for which task type) |

---

## Roadmap

### Completed
- [x] Phase 6 — Cross-platform Flutter dashboard
- [x] Rust workspace with core, agents, network, sandbox crates
- [x] Licensing engine (first-IP-free + Stripe billing)
- [x] Firecracker sandbox detonation
- [x] Threat Hunter with live feed polling

### Phase 2 — Triton Model Integration (active)
- [ ] Define `.triton` binary artifact format (JSON header + packed weights) — **Triton side**
- [ ] Implement `triton_runtime` NumPy/CPU inference engine — **Triton side**
- [ ] Add `TernaryRuntime::load` + `infer` to `crates/core/src/triton.rs` — **BUNNY side**
- [ ] Wire `threat_classifier` model into Threat Hunter agent — **BUNNY side**
- [ ] Wire `packet_filter` model into IoT Firewall agent — **BUNNY side**
- [ ] Implement per-inference telemetry reporting to AI-PORTAL — **BUNNY side**
- [ ] Register BUNNY workers with super-duper-spork via gRPC capability declaration — **BUNNY side**
- [ ] `model_metadata.json` schema validation on artifact load — **BUNNY side**

### Phase 3 — Production Hardening
- [ ] AI Guardian — super-duper-spork traffic monitoring with `traffic_sentinel` model
- [ ] Learning & Adaptation — on-device federated retraining via Triton QAT hooks
- [ ] Rust ↔ Flutter FFI via `flutter_rust_bridge`
- [ ] Production Stripe subscription flows
- [ ] Multi-platform release (macOS, Windows, Linux, Android, iOS)

### Phase 4 — Edge Scale
- [ ] Full fleet deployment (Raspberry Pi, routers, smart TVs)
- [ ] Worker fleet coordination via super-duper-spork
- [ ] On-device model update pipeline (Triton → AI-PORTAL → BUNNY worker)
- [ ] Federated learning across BUNNY worker fleet

---

## Shapeshifter Architecture — Three-Layer Model

This repository is part of the **Shapeshifter Orchestration Model**, a three-layer architecture for adaptive multi-agent workflows, distributed execution, and compression-aware model routing.

```
Adaptive Layer        → Shapeshifter (system-wide orchestration model)
Control Plane         → super-duper-spork + Orchestra + AI-PORTAL
Execution Plane       → Triton + BUNNY + distributed workers
```

### Role of `BUNNY`

**Layer:** Execution Plane

BUNNY is the **distributed worker runtime** in the Execution Plane.

Responsibilities within the Shapeshifter architecture:

- secure task execution on Codespaces, containers, and edge devices
- loading and running Triton-compiled ternary model artifacts
- execution isolation (Firecracker microVM sandboxing)
- edge inference deployment with CPU-first fallback
- worker self-validation and per-inference confidence reporting
- heartbeat and capability declaration to super-duper-spork

### Repository Responsibility Matrix

| Layer     | Component        | Repository          | Role                            |
| --------- | ---------------- | ------------------- | ------------------------------- |
| Adaptive  | Routing & Policy | `ProbFlow`          | uncertainty scoring and routing |
| Control   | Workflow DSL     | `Orchestra`         | task graph definition           |
| Control   | Swarm Runtime    | `super-duper-spork` | scheduling and orchestration    |
| Execution | Model Runtime    | `Triton`            | AI inference and compression    |
| Execution | Worker Runtime   | `BUNNY`             | distributed execution           |
| Interface | UI & Telemetry   | `AI-PORTAL`         | monitoring and user interaction |

### Execution Lifecycle

```
Task Input → Task Classification → Workflow Selection → Task Graph Compilation
    → Worker Dispatch → Model Execution → Validation → Result Synthesis
```

### Telemetry Feedback Loop

```
Triton runtime metrics → AI-PORTAL dashboards → ProbFlow routing models → Shapeshifter policy updates
```

### Goals & Roadmap

See the [Orchestra README](https://github.com/financecommander/Orchestra#architecture-overview-shapeshifter-orchestration-model) for the full architecture specification, goals, and 7-phase roadmap.

> **Engineering principle:** Use the smallest reliable model and workflow capable of completing the task safely.

## Competitive Projection: System-Level Intelligence

The Shapeshifter architecture competes with frontier LLM systems on **system-level capability**, not single-model capability.

**BUNNY role:** Execution Plane — Distributed Worker Runtime

BUNNY's distributed worker architecture is the foundation of the parallel execution advantage. Edge inference, secure task execution, and worker telemetry enable the system to run many specialist models simultaneously — the core mechanism that outperforms sequential frontier reasoning.

### System Benchmark Matrix

| Category | Frontier LLM | Shapeshifter Projection |
|---|---|---|
| Coding | very strong | equal or better |
| Large repo analysis | weak | stronger |
| Structured reasoning | strong | equal |
| Operational automation | weak | stronger |
| Research synthesis | strong | equal |
| Cost efficiency | weak | stronger |
| Scalability | limited | far stronger |

### Advantage Mechanisms

| Mechanism | Frontier LLM | Shapeshifter |
|---|---|---|
| Reasoning | sequential | parallel workers |
| Task decomposition | single-pass | explicit planner |
| Validation | self-consistency | layered (static → tests → review → human) |
| Compute | massive GPUs | compressed + ternary + edge workers |

### Simulation Projection

| Metric | Frontier | Shapeshifter |
|---|---|---|
| Reasoning depth | 95 | 85 |
| Parallel execution | 30 | 95 |
| Validation reliability | 60 | 90 |
| Cost efficiency | 40 | 90 |
| Scalability | 35 | 95 |

Frontier models win on deep reasoning. Shapeshifter wins on system-level capability.

> **Shapeshifter is not a model competitor. It is an orchestration architecture that multiplies model capability.**

See the [Orchestra README](https://github.com/financecommander/Orchestra#competitive-projection-system-level-intelligence) for the full competitive projection, benchmark categories, demonstration strategy, and long-term advantage analysis.


---

## License

**AGPL-3.0-or-later** — See [LICENSE](LICENSE) for details.

---

<p align="center">
  <strong>🐰 BUNNY sees everything. BUNNY protects everything.</strong><br/>
  <em>Powered by Triton ternary AI · Guarding the super-duper-spork Swarm</em>
</p>
