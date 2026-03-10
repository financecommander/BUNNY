# Swarm Data Governance — BUNNY

**Calculus Ecosystem • Governance Rule 4 Compliance**

## Rule

> **super-duper-spork retains ALL swarm management data.**
>
> External repos that generate, test, or benchmark swarm algorithms
> must push assessment results back to `financecommander/super-duper-spork`.

## This Repo's Obligations

BUNNY is the edge worker / security layer that executes swarm tasks.
When edge execution produces algorithm performance telemetry, agent
reliability data, or swarm routing feedback, those must be reported.

| Obligation | Target | Format |
|-----------|--------|--------|
| Push edge swarm execution telemetry | `super-duper-spork/swarm/assessments/` | `bunny_{description}_{YYYY-MM-DD}.md` |
| Report worker-level algorithm success/failure rates | `super-duper-spork/swarm/assessments/` | Assessment markdown |

## Canonical Source of Truth

The single source of truth for all swarm state is:

    financecommander/super-duper-spork

This repo executes tasks at the edge. super-duper-spork retains all swarm
management data, assessment results, and cross-repo totals.

---
*Governance Rule 4 — established 2026-03-09*
