param(
    [string]$Node = "sw-gpu-core-01",
    [string]$Zone = "us-east1-b",
    [string]$NodeRole = "gpu-core",
    [string]$CheckpointRoot = ""
)

$script = @'
set -euo pipefail
sudo bash /tmp/install_swarm_remote_metrics.sh "__NODE_ROLE__" "__CHECKPOINT_ROOT__"
systemctl is-active prometheus-node-exporter
systemctl is-enabled prometheus-node-exporter
systemctl is-active swarm-node-runtime-metrics.timer
curl -fsS http://127.0.0.1:9100/metrics | grep -E 'swarm_checkpoint_best_files|swarm_node_runtime_contract_info' || true
'@

gcloud compute scp infra/provisioning/linux/install_swarm_remote_metrics.sh "${Node}:/tmp/install_swarm_remote_metrics.sh" --zone $Zone --tunnel-through-iap | Out-Null
$script = $script.Replace('__NODE_ROLE__', $NodeRole).Replace('__CHECKPOINT_ROOT__', $CheckpointRoot)
gcloud compute ssh $Node --zone $Zone --tunnel-through-iap --command $script
