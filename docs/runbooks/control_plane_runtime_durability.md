# Control Plane Safety and Runtime Durability

Date: 2026-04-05  
Status: active

## Scope

This runbook covers the immediate hardening steps for:

- `sw-mainframe-01` disk discipline
- `sw-gpu-mee-01` and `sw-gpu-mee-02` MEE scheduler supervision
- post-change health checks
- reboot validation guidance

## Mainframe Disk Discipline

Use:

- `scripts/ops/audit_swarm_mainframe_disk.ps1`
- `scripts/ops/apply_swarm_mainframe_disk_guard.ps1`

The guard installs:

- a dedicated `swarm-mainframe-disk-guard.service`
- a `swarm-mainframe-disk-guard.timer`
- log rotation for `/srv/swarm/logs/*.log`
- journald vacuuming
- safe Docker image and builder pruning
- MEE checkpoint retention against `/srv/swarm/apps/swarm-mainframe/model_research/experiments`

## MEE Scheduler Supervision

Use:

- `scripts/ops/apply_mee_gpu_scheduler_supervision.ps1`

This un-masks, enables, and starts:

- `mee-gpu-worker.service` on `sw-gpu-mee-01`
- `mee-gpu-worker.service` on `sw-gpu-mee-02`

The service reads node-specific settings from:

- `/etc/swarm/mee-gpu.env`

## Health Checks

Use:

- `scripts/ops/check_control_plane_runtime_health.ps1`

This verifies:

- mainframe root disk percentage
- disk guard timer enablement/activity
- MEE scheduler service enablement/activity
- presence of live `mee_scheduler.py` processes

## Reboot Validation

Do not reboot control or MEE nodes casually during an active workload window.

Recommended sequence:

1. run `check_control_plane_runtime_health.ps1`
2. reboot one MEE node during a controlled window
3. verify:
   - node returns to WireGuard/fabric
   - `mee-gpu-worker.service` is active
   - `pgrep -af mee_scheduler.py` returns one supervised process
4. repeat for the second MEE node
5. only then schedule a controlled reboot validation for `sw-mainframe-01`

Mainframe reboot validation must include:

- `sw-mainframe-01` reachable over SSH
- Redpanda reachable on `10.142.0.9:9092`
- collector and control-plane services active
- disk guard timer active after boot

## Bottom Line

The hardening goal is:

- protect `sw-mainframe-01` from disk creep
- make MEE schedulers supervised and restartable
- detect both problems quickly with one health check
