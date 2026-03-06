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
- loading and running Triton-compressed models
- execution isolation and worker telemetry
- edge inference deployment
- worker self-validation and confidence reporting

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
