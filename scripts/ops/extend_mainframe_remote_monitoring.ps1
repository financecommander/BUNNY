param(
    [string]$MainframeNode = "sw-mainframe-01",
    [string]$Zone = "us-east1-b",
    [string]$RemoteNode = "sw-gpu-core-01",
    [string]$RemoteTarget = "10.142.0.6:9100",
    [int]$CheckpointBudget = 0
)

$remoteScript = @'
python3 - <<'PY'
from pathlib import Path
import re


prom = Path("/srv/swarm/apps/swarm-mainframe/deploy/prometheus.yml")
alerts = Path("/srv/swarm/apps/swarm-mainframe/deploy/alerts.yml")
prom_text = prom.read_text()
alerts_text = alerts.read_text()

remote_target = "__REMOTE_TARGET__"
remote_node = "__REMOTE_NODE__"
node_slug = remote_node.replace("-", "_")
group_name = f"remote_{node_slug}"
checkpoint_budget = __CHECKPOINT_BUDGET__


def build_target_block(existing_targets: dict[str, str]) -> str:
    entries = []
    for node, target in sorted(existing_targets.items()):
        entries.append(
            f'      - targets: ["{target}"]\n'
            f'        labels:\n'
            f'          node: "{node}"\n'
            f'          service: "remote-node"\n'
        )
    return '  - job_name: "swarm-remote-node"\n    static_configs:\n' + "".join(entries)


job_pattern = re.compile(
    r'(?ms)^  - job_name: "swarm-remote-node"\n(?:    .*\n|      .*\n)*?(?=^  - job_name: |\Z)'
)
target_pattern = re.compile(
    r'      - targets: \["(?P<target>[^"]+)"\]\n'
    r'        labels:\n'
    r'          node: "(?P<node>[^"]+)"\n'
    r'          service: "remote-node"\n'
)

existing_targets: dict[str, str] = {}
job_match = job_pattern.search(prom_text)
if job_match:
    for target_match in target_pattern.finditer(job_match.group(0)):
        existing_targets[target_match.group("node")] = target_match.group("target")

existing_targets[remote_node] = remote_target
new_job_block = build_target_block(existing_targets)

if job_match:
    prom_text = prom_text[:job_match.start()] + new_job_block + prom_text[job_match.end():]
else:
    prom_text = prom_text.rstrip() + "\n\n" + new_job_block + "\n"

root_warning = f"SwRootDiskWarning_{node_slug}"
root_critical = f"SwRootDiskCritical_{node_slug}"
checkpoint_alert = f"SwCheckpointBudgetExceeded_{node_slug}"

alert_block = f"""
  - name: {group_name}
    rules:
      - alert: {root_warning}
        expr: (1 - node_filesystem_avail_bytes{{job="swarm-remote-node",node="{remote_node}",mountpoint="/",fstype!~"tmpfs|overlay"}} / node_filesystem_size_bytes{{job="swarm-remote-node",node="{remote_node}",mountpoint="/",fstype!~"tmpfs|overlay"}}) * 100 > 75
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "{remote_node} root disk above 75%"
          description: '{remote_node} root usage is {{{{ $value | printf "%.1f" }}}}%.'

      - alert: {root_critical}
        expr: (1 - node_filesystem_avail_bytes{{job="swarm-remote-node",node="{remote_node}",mountpoint="/",fstype!~"tmpfs|overlay"}} / node_filesystem_size_bytes{{job="swarm-remote-node",node="{remote_node}",mountpoint="/",fstype!~"tmpfs|overlay"}}) * 100 > 85
        for: 2m
        labels:
          severity: critical
        annotations:
          summary: "{remote_node} root disk above 85%"
          description: '{remote_node} root usage is {{{{ $value | printf "%.1f" }}}}%.'

      - alert: {checkpoint_alert}
        expr: swarm_checkpoint_best_files{{job="swarm-remote-node",node="{remote_node}"}} > {checkpoint_budget}
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "{remote_node} checkpoint budget exceeded"
          description: '{remote_node} has {{{{ $value | printf "%.0f" }}}} best.pt checkpoints (budget {checkpoint_budget}).'
"""

lines = alerts_text.splitlines()
group_indexes = [idx for idx, line in enumerate(lines) if line.startswith("  - name: ")]
if group_indexes:
    prefix = lines[:group_indexes[0]]
    kept_segments: list[list[str]] = []
    for idx, start in enumerate(group_indexes):
        end = group_indexes[idx + 1] if idx + 1 < len(group_indexes) else len(lines)
        segment = lines[start:end]
        name = segment[0].split(":", 1)[1].strip()
        if name in {group_name, "remote-gpu-core"}:
            continue
        kept_segments.append(segment)
    rebuilt = prefix[:]
    for segment in kept_segments:
        rebuilt.extend(segment)
    if rebuilt and rebuilt[-1] != "":
        rebuilt.append("")
    rebuilt.extend(alert_block.strip("\n").splitlines())
    alerts_text = "\n".join(rebuilt).rstrip() + "\n"
else:
    alerts_text = "groups:\n" + alert_block + "\n"

prom.write_text(prom_text)
alerts.write_text(alerts_text)
PY

sudo docker exec swarm-prometheus promtool check config /etc/prometheus/prometheus.yml
sudo docker exec swarm-prometheus promtool check rules /etc/prometheus/alerts.yml
sudo docker restart swarm-prometheus swarm-alertmanager
'@

$remoteScript = $remoteScript.Replace('__REMOTE_NODE__', $RemoteNode).
    Replace('__REMOTE_TARGET__', $RemoteTarget).
    Replace('__CHECKPOINT_BUDGET__', $CheckpointBudget.ToString())

gcloud compute ssh $MainframeNode --zone $Zone --tunnel-through-iap --command $remoteScript
