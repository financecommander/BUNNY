#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="${1:-/srv/swarm/repo}"
MAINFRAME_BASE="${2:-/srv/swarm/apps/swarm-mainframe}"
EXPERIMENTS_DIR="$MAINFRAME_BASE/model_research/experiments"
LOG_DIR="/srv/swarm/logs"
STATE_DIR="/srv/swarm/state/observability"

mkdir -p "$LOG_DIR" "$STATE_DIR"

install -m 0755 "$REPO_ROOT/scripts/swarm_mee_cleanup.sh" /usr/local/bin/swarm_mee_cleanup.sh
install -m 0755 "$REPO_ROOT/scripts/ops/mee_checkpoint_retention.py" /usr/local/bin/mee_checkpoint_retention.py
install -m 0644 "$REPO_ROOT/infra/logrotate/swarm-mainframe.conf" /etc/logrotate.d/swarm-mainframe
install -m 0644 "$REPO_ROOT/infra/systemd/swarm-mainframe-disk-guard.service" /etc/systemd/system/swarm-mainframe-disk-guard.service
install -m 0644 "$REPO_ROOT/infra/systemd/swarm-mainframe-disk-guard.timer" /etc/systemd/system/swarm-mainframe-disk-guard.timer

cat >/usr/local/bin/swarm-mainframe-disk-guard <<EOF
#!/usr/bin/env bash
set -euo pipefail

EXPERIMENTS_DIR="${EXPERIMENTS_DIR}"
STATE_DIR="${STATE_DIR}"
LOG_FILE="${LOG_DIR}/mainframe_disk_guard.log"
mkdir -p "\$STATE_DIR"

log() {
  printf '[swarm-mainframe-disk-guard] %s\n' "\$*" | tee -a "\$LOG_FILE"
}

log "starting"

if command -v journalctl >/dev/null 2>&1; then
  journalctl --vacuum-size=1G >/dev/null 2>&1 || true
fi

if command -v logrotate >/dev/null 2>&1; then
  logrotate -f /etc/logrotate.d/swarm-mainframe >/dev/null 2>&1 || true
fi

if command -v docker >/dev/null 2>&1; then
  docker image prune -af --filter until=168h >/dev/null 2>&1 || true
  docker builder prune -af --filter until=168h >/dev/null 2>&1 || true
fi

python3 /usr/local/bin/mee_checkpoint_retention.py \\
  --experiments-dir "\$EXPERIMENTS_DIR" \\
  --recent-hours 12 \\
  --min-newest 200 \\
  --apply >"\$STATE_DIR/mainframe_checkpoint_retention_last.json" 2>>"\$LOG_FILE" || true

python3 - <<'PY' >"\$STATE_DIR/mainframe_disk_guard_last.json"
import json
import subprocess
from pathlib import Path

def run(cmd):
    try:
        return subprocess.check_output(cmd, text=True, stderr=subprocess.DEVNULL).strip()
    except subprocess.CalledProcessError:
        return "unavailable"

payload = {
    "root_df": run(["df", "-h", "/"]),
    "srv_logs": run(["du", "-sh", "/srv/swarm/logs"]),
    "mainframe_model_research": run(["du", "-sh", "${MAINFRAME_BASE}/model_research"]),
    "journal_usage": run(["journalctl", "--disk-usage"]),
}
Path("${STATE_DIR}").mkdir(parents=True, exist_ok=True)
print(json.dumps(payload, indent=2))
PY

log "completed"
EOF
chmod 0755 /usr/local/bin/swarm-mainframe-disk-guard

systemctl daemon-reload
systemctl unmask swarm-mainframe-disk-guard.service >/dev/null 2>&1 || true
systemctl enable swarm-mainframe-disk-guard.timer >/dev/null
systemctl restart swarm-mainframe-disk-guard.timer

echo "mainframe disk guard installed"
