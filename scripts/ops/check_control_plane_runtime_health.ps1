param(
    [switch]$Detailed
)

$checks = @(
    @{ Name = "sw-mainframe-01"; Zone = "us-east1-b"; Command = @'
hostnamectl --static
df -P / | tail -n 1
docker ps --format "{{.Names}}" | grep '^swarm-redis$'
systemctl is-active swarm-redis-proxy.service
systemctl is-enabled swarm-redis-proxy.service
systemctl is-failed redis-server.service || true
'@ },
    @{ Name = "sw-gpu-core-01"; Zone = "us-east1-b"; Command = @'
hostnamectl --static
df -P / | tail -n 1
systemctl is-active swarm-checkpoint-retention.timer
systemctl is-enabled swarm-checkpoint-retention.timer
systemctl is-active mee-scheduler.service
grep -n 'MEE_SAVE_BEST_CHECKPOINTS' /etc/systemd/system/mee-scheduler.service /etc/systemd/system/mee-scheduler.service.d/* 2>/dev/null || true
grep -n 'ARCHIVE_' /etc/systemd/system/swarm-checkpoint-retention.service 2>/dev/null || true
ssh -i ~/.ssh/google_compute_engine -o StrictHostKeyChecking=accept-new -o BatchMode=yes -o ConnectTimeout=5 Crypt@sw-data-01 'test -d /data/mee-canonical/sw-gpu-core-01/checkpoints-archive && echo archive_sink_ok' || true
systemctl is-failed logrotate.service || true
python3 - <<'PY'
from pathlib import Path
root = Path("/srv/swarm/apps/mee/model_research/experiments/checkpoints")
files = [p for p in root.rglob("best.pt") if p.is_file()]
print(f"checkpoint_count={len(files)}")
PY
'@ },
    @{ Name = "sw-gpu-code-01"; Zone = "us-east1-b"; Command = @'
hostnamectl --static
systemctl is-enabled nvidia-fabricmanager.service || true
systemctl is-active nvidia-fabricmanager.service || true
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv,noheader
'@ },
    @{ Name = "sw-gpu-a100-01"; Zone = "us-central1-b"; Command = @'
hostnamectl --static
systemctl is-enabled nvidia-fabricmanager.service || true
systemctl is-active nvidia-fabricmanager.service || true
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv,noheader
'@ }
)

foreach ($check in $checks) {
    Write-Host "=== $($check.Name) ==="
    gcloud compute ssh $check.Name --zone $check.Zone --tunnel-through-iap --command $check.Command
    Write-Host ""
}
