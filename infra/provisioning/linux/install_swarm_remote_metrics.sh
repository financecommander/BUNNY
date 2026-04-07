#!/usr/bin/env bash
set -euo pipefail

NODE_ROLE="${1:-general}"
CHECKPOINT_ROOT="${2:-}"
NODE_NAME="${SWARM_NODE_NAME:-$(hostname -s)}"
TEXTFILE_DIR="/var/lib/prometheus/node-exporter"

export DEBIAN_FRONTEND=noninteractive
if ! command -v prometheus-node-exporter >/dev/null 2>&1; then
  apt-get -o Acquire::ForceIPv4=true update -y
  apt-get -o Acquire::ForceIPv4=true install -y prometheus-node-exporter
fi

install -d -m 0755 "$TEXTFILE_DIR" /usr/local/sbin

cat >/usr/local/sbin/swarm-node-runtime-metrics.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

TEXTFILE_DIR="${TEXTFILE_DIR:-/var/lib/prometheus/node-exporter}"
NODE_NAME="${NODE_NAME:-$(hostname)}"
CHECKPOINT_ROOT="${CHECKPOINT_ROOT:-}"
tmp="$(mktemp)"

{
  echo "# HELP swarm_node_runtime_contract_info Static runtime contract marker"
  echo "# TYPE swarm_node_runtime_contract_info gauge"
  echo "swarm_node_runtime_contract_info{node=\"${NODE_NAME}\",role=\"${NODE_ROLE:-general}\"} 1"

  if [[ -n "$CHECKPOINT_ROOT" && -d "$CHECKPOINT_ROOT" ]]; then
    count="$(find "$CHECKPOINT_ROOT" -name best.pt | wc -l)"
    echo "# HELP swarm_checkpoint_best_files Number of best.pt checkpoint files"
    echo "# TYPE swarm_checkpoint_best_files gauge"
    echo "swarm_checkpoint_best_files{node=\"${NODE_NAME}\"} ${count}"
  fi
} >"$tmp"

install -D -m 0644 "$tmp" "$TEXTFILE_DIR/swarm_runtime.prom"
rm -f "$tmp"
EOF
chmod 0755 /usr/local/sbin/swarm-node-runtime-metrics.sh

cat >/etc/default/prometheus-node-exporter <<EOF
ARGS="--collector.textfile.directory=$TEXTFILE_DIR"
EOF

cat >/etc/systemd/system/swarm-node-runtime-metrics.service <<EOF
[Unit]
Description=Emit SWARM runtime metrics for node exporter
After=network-online.target

[Service]
Type=oneshot
Environment=NODE_ROLE=$NODE_ROLE
Environment=NODE_NAME=$NODE_NAME
Environment=CHECKPOINT_ROOT=$CHECKPOINT_ROOT
Environment=TEXTFILE_DIR=$TEXTFILE_DIR
ExecStart=/usr/local/sbin/swarm-node-runtime-metrics.sh
EOF

cat >/etc/systemd/system/swarm-node-runtime-metrics.timer <<'EOF'
[Unit]
Description=Refresh SWARM runtime metrics

[Timer]
OnBootSec=2m
OnUnitActiveSec=2m
Unit=swarm-node-runtime-metrics.service

[Install]
WantedBy=timers.target
EOF

systemctl daemon-reload
systemctl enable --now prometheus-node-exporter
systemctl restart prometheus-node-exporter
systemctl enable --now swarm-node-runtime-metrics.timer
systemctl start swarm-node-runtime-metrics.service

echo "swarm remote metrics installed node=$NODE_NAME role=$NODE_ROLE checkpoint_root=$CHECKPOINT_ROOT"
