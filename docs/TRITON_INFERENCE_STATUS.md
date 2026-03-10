# Triton Rust CUDA Backend — Integration Status

**Upstream**: [`financecommander/Triton@b0ef84f`](https://github.com/financecommander/Triton/commit/b0ef84f)
**Date**: 2026-03-09

---

## Rust CUDA Inference Backend (triton-rs)

The Triton repo now ships a production-grade **Rust CUDA inference server** (`triton-rs`) that BUNNY edge workers can use as the native inference backend. This replaces the ONNX/Python path with a pure Rust + CUDA pipeline.

### What Changed

| Component | Before | After |
|-----------|--------|-------|
| **Inference runtime** | ONNX (ort crate) or Python Triton | Native Rust + candle + CUDA PTX |
| **cudarc version** | 0.12 | 0.13 (matches candle-core 0.8) |
| **Model tiers** | 10 (cell → large) | 15 (cell_pico → brain_a) |
| **GQA attention** | Untested | Fixed (contiguity before matmul) |
| **softmax on CUDA** | Broken | Custom `softmax_last_dim_compat()` |
| **Binary size** | N/A | 11 MB (release, CUDA-enabled) |

### Benchmark Validated

A/B benchmark on NVIDIA L4 24GB confirms:
- **15/15 model tiers** serving (Rust) vs 12/15 (NVIDIA Triton)
- **502 tok/s** single-stream throughput
- **1.5 GB VRAM** usage (vs 11.8 GB for NVIDIA Triton)
- **5.8x cold-start advantage** on complex architectures (deep_narrow, gqa)

### BUNNY Integration Path

BUNNY's `ternary_engine.rs` (Phase 2 native engine) can now be validated against the same model weights used by `triton-rs`. The key shared primitives:

```
Triton repo (triton-rs):
  backend/rust/src/model/ternary_llama.rs    — TernaryLlama transformer
  backend/rust/src/ops/ternary_matmul.rs     — Fused 2-bit matmul + CUDA PTX
  backend/rust/src/ops/packed_ternary.rs     — 2-bit weight packing

BUNNY repo:
  crates/agents/src/ternary_engine.rs        — Phase 2 native Rust engine
  crates/agents/src/triton_agent.rs          — ONNX model loading (Phase 1)
```

### Contiguity Fix (Critical for BUNNY)

If BUNNY's ternary engine implements GQA (grouped-query attention) with `repeat_kv()`, it must ensure `.contiguous()` is called after transpose/expand before any matmul:

```rust
// In attention: ensure contiguous before matmul
let scores = q.contiguous()?.matmul(&k_expanded.t()?.contiguous()?)?;
let output = attn_weights.matmul(&v_expanded.contiguous()?)?;

// In repeat_kv: ensure contiguous after reshape
fn repeat_kv(x: &Tensor, n_rep: usize) -> Result<Tensor> {
    // ... expand + reshape ...
    result.contiguous()
}
```

### Next Steps

1. Validate BUNNY's Phase 2 ternary engine weights against triton-rs model repos
2. Port the `softmax_last_dim_compat()` workaround if BUNNY targets candle CUDA
3. Consider sharing the fused CUDA PTX kernels from `kernels/cuda/` for edge GPU inference

### Full Report

See [`Triton/benchmarks/AB_REPORT.md`](https://github.com/financecommander/Triton/blob/main/benchmarks/AB_REPORT.md).
