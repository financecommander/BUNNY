param(
    [string]$Node = "sw-gpu-core-01",
    [string]$Zone = "us-east1-b",
    [string]$OrchestratorPath = "/srv/swarm/apps/mee/model_research/training/orchestrator.py",
    [string]$ServiceName = "mee-scheduler.service",
    [int]$SaveBestCheckpoints = 0
)

gcloud compute scp infra/provisioning/linux/apply_mee_checkpoint_policy.py "${Node}:/tmp/apply_mee_checkpoint_policy.py" --zone $Zone --tunnel-through-iap | Out-Null

$remote = @'
set -euo pipefail
sudo install -m 0755 /tmp/apply_mee_checkpoint_policy.py /usr/local/bin/apply_mee_checkpoint_policy.py
sudo python3 /usr/local/bin/apply_mee_checkpoint_policy.py "__ORCHESTRATOR_PATH__"
sudo install -d -m 0755 /etc/systemd/system/__SERVICE_NAME__.d
sudo tee /etc/systemd/system/__SERVICE_NAME__.d/checkpoint-policy.conf >/dev/null <<'EOF'
[Service]
Environment=MEE_SAVE_BEST_CHECKPOINTS=__SAVE_BEST__
EOF
sudo systemctl daemon-reload
sudo systemctl restart __SERVICE_NAME__
grep -n -C 2 'MEE_SAVE_BEST_CHECKPOINTS' "__ORCHESTRATOR_PATH__"
sudo cat /etc/systemd/system/__SERVICE_NAME__.d/checkpoint-policy.conf
sudo systemctl is-active __SERVICE_NAME__
'@

$remote = $remote.Replace('__ORCHESTRATOR_PATH__', $OrchestratorPath).
    Replace('__SERVICE_NAME__', $ServiceName).
    Replace('__SAVE_BEST__', $SaveBestCheckpoints.ToString())

gcloud compute ssh $Node --zone $Zone --tunnel-through-iap --command $remote
