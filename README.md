<p align="center">
  <img src="https://img.shields.io/badge/BUNNY-AI%20Machine%20Defender-ff4500?style=for-the-badge&amp;logo=rust&amp;logoColor=white" alt="BUNNY Badge"/>
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
                       │  gRPC / mTLS (planned)
┌──────────────────────▼──────────────────────────────────────┐
│                        🐰 BUNNY                             │
│  ┌──────────┐ ┌──────────┐ ┌─────────┐ ┌────────────────┐  │
│  │  Threat  │ │    AI    │ │ Sandbox │ │  IoT Firewall  │  │
│  │  Hunter  │ │ Guardian │ │  Agent  │ │     Agent      │  │
│  └────┬─────┘ └────┬─────┘ └────┬────┘ └───────┬────────┘  │
│       │            │            │               │           │
│       └────────────┴────────────┴───────────────┘           │
│              Triton Pure Agent Runtime (Rust)                │
│          (every agent is a Triton ternary agent)            │
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
|-------|------|--------------|
| **🔍 Threat Hunter** | Polls live threat feeds (Abuse.ch, OTX), correlates IOCs against local network traffic via vector similarity search (LanceDB). | `threat_classifier` |
| **🛡️ AI Guardian** | Monitors all traffic flowing to/from super-duper-spork (SendGrid webhooks, GHL API calls, LLM inference requests). Detects prompt injection, data exfiltration, and API abuse. 32-dim features, 6 threat categories, rate limiting. | `traffic_sentinel` |
| **💣 Sandbox Agent** | Detonates suspicious payloads in Firecracker microVMs. Verdicts returned with confidence scores. | `malware_classifier` |
| **📡 IoT Firewall** | Deep packet inspection at the edge for IoT/smart devices. Blocks C2 callbacks, DNS tunneling, and lateral movement. 24-dim packet features. | `packet_filter` |
| **🧠 Learning &amp; Adaptation** | Continuously retrains local ternary models from new threat data without cloud dependency. Federated learning with gradient accumulation and re-quantization. | `anomaly_detector` |

---

## Project Structure

````text
BUNNY/
├── Cargo.toml                 # Rust workspace root (8 crates)
├── .env.example               # Environment variables template
├── crates/
│   ├── crypto/                # bunny-crypto: encrypted swarm communications (109 tests)
│   │   └── src/
│   │       ├── cipher.rs          # AES-256-GCM / ChaCha20-Poly1305 (auto-detect AES-NI)
│   │       ├── key_exchange.rs    # Hybrid PQ key exchange (X25519 + ML-KEM-768)
│   │       ├── hybrid_sign.rs     # Ed25519 + ML-DSA-65 dual signatures (FIPS 204)
│   │       ├── transport.rs       # Authenticated node-to-node transport + session expiry
│   │       ├── session.rs         # Session store + 128-bit sliding window replay protection
│   │       ├── envelope.rs        # SwarmEnvelope — 62-byte AAD-bound wire format
│   │       ├── cloaked.rs         # CloakedTransport — metadata minimization, cover traffic
│   │       ├── identity.rs        # Node identity (Ed25519 + ML-KEM-768 keypairs)
│   │       ├── execution.rs       # Decrypt-at-boundary for Triton inference
│   │       ├── gateway.rs         # Inference gateway — decrypt/re-encrypt
│   │       ├── artifact.rs        # Signed artifact manifest chain
│   │       ├── swarm.rs           # 16 typed encrypted messages + role enforcement
│   │       ├── ternary.rs         # TernaryPacket — 7 payload types
│   │       └── kdf.rs             # HKDF-SHA256 key derivation
│   ├── triton/                # bunny-triton: ternary neural inference engine (44 tests)
│   │   └── src/
│   │       ├── engine.rs          # Inference engine — forward pass + argmax prediction
│   │       ├── model.rs           # Model builder — layer chain validation
│   │       ├── layer.rs           # 2-bit packed matmul, zero-skipping, fused batch-norm
│   │       ├── packing.rs         # Ternary weight packing (16 values per u32)
│   │       ├── activation.rs      # ReLU, Sigmoid, Softmax, None
│   │       ├── bunny_format.rs    # .bunny binary model format (CRC32 + mmap)
│   │       ├── shard.rs           # Model shard split/merge for distributed inference
│   │       └── safetensors_loader.rs  # SafeTensors → ternary conversion
│   ├── network/               # bunny-network: QUIC transport + orchestration (37 tests)
│   │   └── src/
│   │       ├── server.rs          # QUIC server — accept connections, dispatch messages
│   │       ├── client.rs          # QUIC client — connect, handshake, send
│   │       ├── connection.rs      # Handshake initiator/responder over QUIC streams
│   │       ├── framing.rs         # Length-prefixed frame protocol (8 frame types)
│   │       ├── peer.rs            # PeerManager — DashMap concurrent peer tracking
│   │       ├── tls.rs             # Self-signed TLS (app-layer auth via bunny-crypto)
│   │       ├── config.rs          # NetworkConfig (bind addr, timeouts, limits)
│   │       └── orchestration/     # Distributed scheduling
│   │           ├── health.rs      # Health check heartbeat monitor
│   │           ├── scheduler.rs   # Priority queue task scheduler
│   │           └── worker_pool.rs # Worker pool with capability matching
│   ├── calculus/              # bunny-calculus: math primitives (45 tests)
│   │   └── src/
│   │       ├── gradient.rs        # STE, gradient clipping, alignment
│   │       ├── similarity.rs      # Cosine, Jaccard, kNN search
│   │       ├── statistics.rs      # Running stats, EMA, z-score, IQR outliers
│   │       ├── optimizer.rs       # Threshold search, entropy, balance analysis
│   │       ├── symbolic.rs        # Symbolic expression evaluator
│   │       └── tensor_ops.rs      # Dot product, L1 norm, sparsity, saturating ops
│   ├── portal/                # bunny-portal: HTTP API gateway (23 tests)
│   │   └── src/
│   │       ├── api.rs             # Axum router — inference, model CRUD, health
│   │       ├── router.rs          # Model routing + input validation
│   │       ├── model_registry.rs  # Worker-to-model assignment + VRAM estimation
│   │       ├── session_manager.rs # Session lifecycle + expiry eviction
│   │       └── telemetry.rs       # Prometheus-style counters, gauges, latency
│   ├── agents/                # bunny-agents: agent trait + runner (10 tests)
│   │   └── src/
│   │       ├── agent.rs           # Agent trait, AgentScale, ModelScale classification
│   │       ├── runner.rs          # Agent dispatch, register/unregister, inference
│   │       └── config.rs          # Agent configuration (model paths, thresholds)
│   ├── core/                  # bunny-core: protobuf, DB, vectors, licensing
│   │   └── src/
│   │       ├── licensing.rs       # First-IP-free + $1/mo per extra IP (Stripe)
│   │       ├── proto.rs           # gRPC protobuf codegen (tonic-prost-build)
│   │       ├── db.rs              # Database stubs
│   │       └── vector.rs          # Vector store interface
│   └── sandbox/               # bunny-sandbox: Firecracker detonation
│       └── src/
│           ├── firecracker.rs     # MicroVM payload detonation
│           └── threat_hunter.rs   # Live feed polling (Abuse.ch + OTX)
├── flutter/                   # Cross-platform UI (4 screens)
└── README.md
````

---

## Triton Integration

BUNNY's brain runs exclusively on **Triton** — the ternary neural network DSL from [`financecommander/Triton`](https://github.com/financecommander/Triton).

- **All five agents** use Triton ternary models for inference
- **No FP32 models or heavy frameworks** — optimized for constrained hardware

### Integration path

Triton (Python) compiles `.tri` source → PyTorch → ONNX export. BUNNY loads and runs the models in pure Rust:

- **Phase 1** ✅: ONNX runtime (`ort` crate) — load pre-trained ternary ONNX models, zero Python at runtime
- **Phase 2** ✅: Native Rust ternary engine — 2-bit packed matmul, zero-skipping, fused batch-norm, `.bunny` binary format (~450 lines, 10 tests)
- **Phase 3**: Port the `.tri` compiler to Rust — full Triton toolchain in Rust

Phase 2 is implemented in `crates/agents/src/ternary_engine.rs` — a complete native Rust ternary inference engine with 2-bit packed weights, zero-skip matrix multiplication, fused batch normalization, and a compact `.bunny` binary model format.

```rust
// Phase 2: Native Rust ternary inference (no ONNX, no Python)
use bunny_agents::ternary_engine::{TernaryModelBuilder, Activation};

let model = TernaryModelBuilder::new()
    .add_layer(24, 32, Activation::ReLU)   // packet features → hidden
    .add_layer(32, 6, Activation::None)     // hidden → threat categories
    .build();
let verdict = model.forward(&packet_features);
// 2-bit packed weights, zero-skipping, pure Rust, no runtime deps
```

Target hardware:
- 📱 Mobile (iOS / Android)
- 📡 Routers &amp; gateways
- 📺 Smart TVs
- 🍓 Raspberry Pi &amp; embedded Linux

### Triton security models

Triton provides 5 purpose-built ternary classifiers for BUNNY in [`models/security/`](https://github.com/financecommander/Triton/tree/main/models/security):

| Model | BUNNY Agent | Purpose |
|-------|-------------|---------|
| `threat_classifier` | Threat Hunter | IOC correlation and threat categorization |
| `packet_filter` | IoT Firewall | Edge DPI packet classification |
| `malware_classifier` | Sandbox Agent | Payload detonation verdict |
| `traffic_sentinel` | AI Guardian | API traffic monitoring |
| `anomaly_detector` | Learning &amp; Adaptation | Novel threat detection |

These models are compiled through Triton's DSL and exported as ONNX (Phase 1) or loaded via the native Rust ternary engine (Phase 2).

---

## Triton-backed Inference Architecture

**Triton-backed inference** means the swarm routes all model execution through the **Triton inference runtime**, which compiles and executes the model kernels. In your stack this should serve the **ternary compressed model**.

Below is the correct architecture for your system.

---

### System Stack

```text
User / API
   ↓
Encrypted Bunny Swarm
   ↓
Secure Inference Router
   ↓
Triton Inference Runtime
   ↓
Ternary Model Kernels
```

Roles:

| Layer         | Responsibility                    |
| ------------- | --------------------------------- |
| bunny         | encrypted swarm communication     |
| router        | model selection + routing         |
| Triton        | compile and execute kernels       |
| ternary model | compressed inference architecture |

---

### Triton-backed Inference Flow

#### 1. Request enters swarm

Agent creates inference request.

```json
{
  "type": "MODEL_CALL",
  "model": "ternary_llm",
  "payload": "encrypted_prompt_blob"
}
```

---

#### 2. Bunny decrypts at execution boundary

Inside the secure runtime:

```
decrypt → validate → route
```

---

#### 3. Router sends request to Triton

Example endpoint:

```
http://triton-server:8000/v2/models/ternary_llm/infer
```

Payload:

```json
{
  "inputs": [
    {
      "name": "input_ids",
      "datatype": "INT32",
      "shape": [1, 256],
      "data": [...]
    }
  ]
}
```

---

#### 4. Triton executes compiled kernels

Triton handles:

```
model loading
kernel scheduling
GPU execution
tensor memory
```

For your stack it should run:

```
ternary kernels
compressed weights
quantized execution
```

---

#### 5. Result returned to swarm

Triton response:

```json
{
 "outputs":[
   {
     "name":"logits",
     "shape":[1,256,32000]
   }
 ]
}
```

Swarm then:

```
encrypt → send → requesting agent
```

---

### Required Components in the Repo

To truly be **Triton-backed inference**, the system must contain:

#### Model Repository

Typical structure:

```
model_repository/
   ternary_llm/
       config.pbtxt
       1/
         model.pt
```

---

#### Triton Server

Example run command:

```
tritonserver --model-repository=/models
```

---

#### Bunny Inference Client

The swarm must call Triton using:

* HTTP client
* gRPC client
* Triton Python API

Example:

```python
import tritonclient.http as httpclient
client = httpclient.InferenceServerClient("triton-server:8000")
response = client.infer("ternary_llm", inputs)
```

---

### Security Boundary

Your encryption model should decrypt **only at the inference gateway**, not earlier.

Correct boundary:

```text
Encrypted swarm
   ↓
Inference gateway (decrypt)
   ↓
Triton runtime
   ↓
Result
   ↓
Re-encrypt
```

---

### Why Triton Matters for Your Architecture

Triton gives:

• GPU scheduling
• high-throughput inference
• model versioning
• batching
• kernel compilation support

This is critical for a **compressed ternary architecture**, because the runtime must efficiently execute the custom kernels.

---

### Correct Design Statement

Your system should be described as:

```
An encrypted AI swarm whose inference layer is executed through Triton-backed runtime serving ternary compressed models.
```

---

### Next Step (Important)

To fully align with your **compression + quantum-proof architecture**, the next engineering step is:

**compile the ternary kernels directly into Triton runtime paths** rather than using standard PyTorch fallback.

That creates:

```
native ternary GPU kernels
+
compressed weight loading
+
high throughput swarm inference
```

---

If useful, the next thing to map is the **exact runtime pipeline**:

```
swarm → router → Triton → ternary kernels → GPU
```

including where compression, encryption, and batching occur.

---

## Phase 2 Artifact Contract

Interface contract between Triton (producer) and BUNNY (consumer). Triton owns the artifact format; BUNNY owns the loader and runtime binding.

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

---

## Monetization Model

BUNNY uses a **unique per-IP licensing model** — code lives exclusively in this repo.

| Tier | Price | Details |
|------|-------|---------|
| **Free** | $0 forever | First detected IP/device is always protected free |
| **Pro** | $1/month per IP | Each additional IP the user selects via dashboard |

Users choose which IPs to protect through the Flutter dashboard. Billing is handled via **Stripe** (subscriptions auto-created in `bunny-core` licensing module, feature-gated: `cargo build --features stripe`).

---

## Tech Stack

| Layer | Technology |
|-------|------------|
| **Core engine** | Rust 1.85+ (edition 2021) |
| **AI inference** | Triton ternary (Phase 1: ONNX via `ort`, Phase 2: native Rust 2-bit engine) |
| **Cross-platform UI** | Flutter 3.29+ with Rust ↔ Flutter FFI (C ABI bridge) |
| **Database** | Turso (libSQL) — edge-replicated SQLite |
| **Vector store** | LanceDB — local-first vector similarity |
| **Sandbox** | Firecracker microVMs |
| **Billing** | Stripe (subscriptions + webhooks, feature-gated) |
| **Auth** | JWT (`jsonwebtoken` with AWS LC RS) |
| **RPC** | gRPC via Tonic + Prost (protobuf) |
| **Ecosystem** | super-duper-spork (Swarm mainframe), Triton (model compiler) |

---

## Quick Start

### Prerequisites

- Rust ≥ 1.85 (edition 2021)
- Flutter ≥ 3.29
- Firecracker (optional, for sandbox agent)

### Setup

```bash
# Clone
git clone https://github.com/financecommander/BUNNY.git
cd BUNNY

# Configure environment
cp .env.example .env
# Fill in: TURSO_URL, TURSO_AUTH_TOKEN, JWT_SECRET, STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET

# Build Rust workspace
cargo build --release

# Run all tests (268 pass across 8 crates)
cargo test

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
├── super-duper-spork      ← Control Plane: Swarm mainframe, 17 agent castes, task lifecycle
│                            (BUNNY protects this — every outbound API call is audited)
├── Triton                 ← Execution Plane: Ternary NN DSL + compiler + 9-model ladder
│                            (compiles every BUNNY agent model, 5 security classifiers)
├── BUNNY                  ← You are here: Execution Plane edge worker + agent builder
│                            (Rust core, native ternary engine, 5 security agents, Flutter UI)
├── Orchestra              ← Control Plane: Workflow DSL (.orc task graphs)
├── ProbFlow               ← Adaptive Layer: Uncertainty scoring + routing
└── AI-PORTAL              ← Interface: Model registry, eval dashboards, telemetry
````

### Cross-repo data flow

```
Triton compiles .tri models ──► BUNNY loads them (ONNX or native .bunny format)
super-duper-spork dispatches tasks ──► BUNNY agents execute on edge devices
BUNNY telemetry ──► AI-PORTAL dashboards ──► ProbFlow routing updates
Orchestra .orc blueprints ──► super-duper-spork schedules ──► BUNNY executes
```

### Cross-Repo Responsibilities (from BUNNY's perspective)

| Repo | What BUNNY provides | What BUNNY consumes |
|---|---|---|
| **Triton** | Hardware targets, edge constraints, telemetry | Compiled model artifacts (ONNX + `.bunny` native format) |
| **AI-PORTAL** | Per-inference telemetry (latency, confidence, worker ID) | Model registry lookups, deployment instructions |
| **super-duper-spork** | Task execution results, worker availability | Task dispatch, model selection decisions |
| **Orchestra** | Worker status, capability declarations | Workflow graph task assignments |
| **ProbFlow** | Confidence and latency streams | Updated routing weights |

---

## Roadmap

### Completed
- [x] Rust workspace with 8 crates (crypto, triton, network, calculus, portal, agents, core, sandbox)
- [x] **Encrypted swarm comms** — hybrid PQ key exchange (X25519 + ML-KEM-768), dual cipher (AES-256-GCM / ChaCha20-Poly1305), 128-bit sliding window replay protection
- [x] **Security hardening** — IKM zeroization, ZeroizeOnDrop signing keys, session expiry, envelope age anti-delay, forward-secure rekey, traffic policy jitter, peer role auth
- [x] **QUIC transport** — quinn server/client, framing protocol, peer management, distributed orchestration (health, scheduler, worker pool)
- [x] **Ternary inference engine** — 2-bit packed matmul, zero-skipping, fused batch-norm, shard split/merge, safetensors loader, `.bunny` format
- [x] **Math primitives** — gradient ops, cosine/Jaccard similarity, statistics (EMA, z-score, IQR), symbolic evaluator, tensor ops
- [x] **HTTP API gateway** — Axum router, model registry, session manager, Prometheus telemetry
- [x] **Agent framework** — trait-based agents, runner dispatch, config, scale classification
- [x] Turso DB + LanceDB vector store + gRPC codegen + JWT auth
- [x] Security agents — Threat Hunter, AI Guardian, Sandbox, IoT Firewall, Learning &amp; Adaptation
- [x] Flutter dashboard (4 screens) + Rust ↔ Flutter FFI (C ABI bridge)
- [x] Stripe billing (feature-gated) + Firecracker sandbox
- [x] **268 tests** across all 8 crates (crypto: 109, triton: 44, calculus: 45, network: 37, portal: 23, agents: 10)

### In Progress
- [ ] Production Stripe subscription flows
- [ ] Multi-platform release (macOS, Windows, Linux, Android, iOS)

### Planned
- [ ] Bilateral rekey protocol (RekeyRequest/RekeyAccept message pair)
- [ ] Cover traffic background task wired into transport layer
- [ ] Edge telemetry pipeline to AI-PORTAL
- [ ] Port `.tri` compiler to Rust (Phase 3)

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

BUNNY is the **Rust agent builder and edge worker runtime** in the Execution Plane. It exclusively produces Triton pure agents.

**Implemented today:**
- **Build and deploy Triton pure agents** — every agent runs 2-bit ternary inference
- Native Rust ternary engine (Phase 2) — 2-bit packed matmul, zero-skipping, fused batch-norm
- 5 security agents: Threat Hunter, AI Guardian, IoT Firewall, Sandbox, Learning &amp; Adaptation
- ONNX model loading via `ort` crate (Phase 1)
- Federated learning with gradient accumulation and re-quantization
- Flutter cross-platform dashboard (4 screens)
- Rust ↔ Flutter FFI bridge (C ABI)

**Planned (not yet implemented):**
- Distributed worker registration and task dispatch
- gRPC/mTLS secure transport
- Edge telemetry pipeline
- Worker self-validation and confidence reporting
- Fleet health monitoring

### Repository Responsibility Matrix

| Layer     | Component        | Repository          | Role                            |
| --------- | ---------------- | ------------------- | ------------------------------- |
| Adaptive  | Routing &amp; Policy | `ProbFlow`          | uncertainty scoring and routing |
| Control   | Workflow DSL     | `Orchestra`         | task graph definition           |
| Control   | Swarm Runtime    | `super-duper-spork` | scheduling, orchestration, live model routing (Phase 36) |
| Execution | Model Runtime    | `Triton`            | AI inference and compression    |
| Execution | Worker Runtime   | `BUNNY`             | edge inference and security agents (native Rust ternary engine) |
| Interface | UI &amp; Telemetry   | `AI-PORTAL`         | monitoring and user interaction |

### Execution Lifecycle

```
Task Input → Task Classification → Workflow Selection → Task Graph Compilation
    → Worker Dispatch → Model Execution → Validation → Result Synthesis
```

### Telemetry Feedback Loop

```
Triton runtime metrics → AI-PORTAL dashboards → ProbFlow routing models → Shapeshifter policy updates
```

### Goals &amp; Roadmap

See the [Orchestra README](https://github.com/financecommander/Orchestra#architecture-overview-shapeshifter-orchestration-model) for the full architecture specification, goals, and 7-phase roadmap.

> **Engineering principle:** Use the smallest reliable model and workflow capable of completing the task safely.

## Competitive Projection: System-Level Intelligence

The Shapeshifter architecture competes with frontier LLM systems on **system-level capability**, not single-model capability.

**BUNNY role:** Execution Plane — Triton Agent Builder &amp; Worker Runtime

BUNNY builds and runs Triton pure agents — every specialist model is a ternary agent. Edge inference, secure task execution, and worker telemetry enable the system to run many Triton agents simultaneously — the core mechanism that outperforms sequential frontier reasoning.

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
