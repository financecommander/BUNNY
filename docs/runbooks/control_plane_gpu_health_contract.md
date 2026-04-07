# Control Plane and GPU Health Contract

## Canonical expectations

### `sw-mainframe-01`
- Guest hostname should be `sw-mainframe-01`.
- Redis is canonical through the `swarm-redis` container and `swarm-redis-proxy.service`.
- The legacy host `redis-server.service` is not part of the active contract and should remain masked.
- Root disk should stay below `75%`, with intervention required at `85%`.

### `sw-gpu-core-01`
- Root disk should stay below `75%`, with intervention required at `85%`.
- MEE experiment checkpoints are an active working set, not a permanent archive.
- `swarm-checkpoint-retention.timer` must stay enabled and active.
- `logrotate.service` must remain healthy after retention runs.
- `mee-scheduler.service` on this node should run in helper mode without persisting full per-run `best.pt` checkpoints to root storage.
- Current canonical policy on this node is archive-only on root storage (`0` retained locally after pruning).

### Single-GPU GPU nodes
- Current single-GPU fleet:
  - `sw-gpu-core-01`
  - `sw-gpu-code-01`
  - `sw-gpu-code-02`
  - `sw-gpu-embed-01`
  - `sw-gpu-mee-01`
  - `sw-gpu-mee-02`
  - `sw-gpu-voice-01`
  - `sw-gpu-voice-02`
  - `sw-gpu-a100-01`
- `nvidia-fabricmanager.service` is not part of the health contract on these hosts and should remain masked unless a documented NVSwitch requirement is introduced later.
- GPU health is defined by:
  - `nvidia-smi` working
  - expected lane runtime being active
  - disk below threshold

## Operational guidance

### Checkpoint retention
- Keep only the newest working-set checkpoints on `sw-gpu-core-01`.
- Archive or promote winners elsewhere if they must be preserved long-term.
- Do not allow experiment checkpoints to accumulate indefinitely on root storage.
- Current policy target on `sw-gpu-core-01`: archive-only on local root (`0` retained locally after pruning).
- Preferred storage for long-lived artifacts:
  - `/data/swarm/...` on `sw-data-01`
  - object storage
  - another explicit archive path outside root volume
- Current archive sink:
  - `/data/mee-canonical/sw-gpu-core-01/checkpoints-archive` on `sw-data-01`
- Archive transport is best-effort. Node safety wins:
  - if archive transfer succeeds, pruned checkpoints are preserved off root
  - if archive transfer fails, retention still prunes locally to protect `sw-gpu-core-01`

### Scheduler write discipline
- `sw-gpu-core-01` is a helper/staging GPU lane, not the canonical long-term artifact host.
- On this node, checkpoint persistence should be reduced or disabled unless a specific experiment explicitly requires it.
- MEE real-data lanes may continue persisting checkpoints according to their own runtime contract.
- Repo-controlled deployment behavior:
  - `infra/provisioning/linux/apply_mee_checkpoint_policy.py` patches any deployed `model_research/training/orchestrator.py` to honor `MEE_SAVE_BEST_CHECKPOINTS`
  - `infra/provisioning/linux/install_mee_gpu_runtime.sh` now writes `MEE_SAVE_BEST_CHECKPOINTS` into `/etc/swarm/mee-gpu.env`

### Remote metrics and alerting
- Remote GPU/control nodes that matter operationally should run `prometheus-node-exporter` with the textfile collector enabled.
- Current seed implementation:
  - `infra/provisioning/linux/install_swarm_remote_metrics.sh`
  - `scripts/ops/apply_swarm_remote_metrics.ps1`
  - `scripts/ops/extend_mainframe_remote_monitoring.ps1`
- Current remote-monitored nodes:
  - `sw-data-01`
  - `sw-gpu-core-01`
  - `sw-gpu-code-01`
  - `sw-gpu-code-02`
  - `sw-gpu-embed-01`
  - `sw-gpu-mee-01`
  - `sw-gpu-mee-02`
  - `sw-gpu-voice-01`
  - `sw-gpu-voice-02`
  - `sw-gpu-a100-01`
- Policy application for live MEE training nodes:
  - `scripts/ops/apply_live_mee_checkpoint_policy.ps1`
- `sw-gpu-core-01` should expose:
  - root disk metrics via `node-exporter`
  - `swarm_checkpoint_best_files` via the textfile collector

### Redis model on mainframe
- `swarm-redis` container is canonical.
- `swarm-redis-proxy.service` is the canonical internal-IP exposure path.
- Host `redis-server.service` is obsolete and should remain masked.

### Mainframe identity
- Keep `/etc/hosts` transition aliases only as long as they are still needed.
- Prefer the canonical hostname in logs, health checks, and service metadata.

## Verification

Run:

```powershell
pwsh -File "C:\Users\Crypt\OneDrive\Documents\New project\scripts\ops\check_control_plane_runtime_health.ps1"
```
