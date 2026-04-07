param(
    [string]$Node = "sw-gpu-core-01",
    [string]$Zone = "us-east1-b",
    [int]$KeepCount = 0,
    [int]$JournalVacuumMb = 512,
    [string]$ArchiveUser = "Crypt",
    [string]$ArchiveHost = "sw-data-01",
    [string]$ArchivePath = "/data/mee-canonical/sw-gpu-core-01/checkpoints-archive"
)

$script = @'
set -euo pipefail
sudo install -d -m 0755 /usr/local/sbin
sudo tee /usr/local/sbin/swarm-checkpoint-retention.sh >/dev/null <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

CHECKPOINT_ROOT="/srv/swarm/apps/mee/model_research/experiments/checkpoints"
KEEP_COUNT="${KEEP_COUNT:-500}"
JOURNAL_VACUUM_MB="${JOURNAL_VACUUM_MB:-512}"
LOG_DIR="/var/log/swarm"
MANIFEST_DIR="$LOG_DIR/checkpoint-retention"
ARCHIVE_USER="${ARCHIVE_USER:-}"
ARCHIVE_HOST="${ARCHIVE_HOST:-}"
ARCHIVE_PATH="${ARCHIVE_PATH:-}"
ARCHIVE_KEY_PATH="${ARCHIVE_KEY_PATH:-/home/Crypt/.ssh/google_compute_engine}"
ARCHIVE_ENABLED=0

mkdir -p "$MANIFEST_DIR"

if [[ ! -d "$CHECKPOINT_ROOT" ]]; then
  exit 0
fi

if [[ -n "$ARCHIVE_USER" && -n "$ARCHIVE_HOST" && -n "$ARCHIVE_PATH" ]]; then
  ARCHIVE_ENABLED=1
fi

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
manifest="$MANIFEST_DIR/pruned-$timestamp.txt"
archive_log="$MANIFEST_DIR/archive-$timestamp.log"

python3 - <<'PY' >"$manifest"
from pathlib import Path
import os

root = Path("/srv/swarm/apps/mee/model_research/experiments/checkpoints")
keep = int(os.environ.get("KEEP_COUNT", "500"))
files = [p for p in root.rglob("best.pt") if p.is_file()]
files.sort(key=lambda p: p.stat().st_mtime, reverse=True)
for path in files[keep:]:
    print(path)
PY

if [[ -s "$manifest" ]]; then
  if [[ "$ARCHIVE_ENABLED" -eq 1 ]]; then
    remote_dir="$ARCHIVE_PATH/$timestamp"
    archive_tar="$remote_dir/checkpoints.tar.gz"
    archive_manifest="$remote_dir/pruned.txt"
    ssh_opts=(-i "$ARCHIVE_KEY_PATH" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o BatchMode=yes -o ConnectTimeout=10)
    if [[ -r "$ARCHIVE_KEY_PATH" ]]; then
      if ssh "${ssh_opts[@]}" "$ARCHIVE_USER@$ARCHIVE_HOST" "mkdir -p '$remote_dir'"; then
        if tar --absolute-names -czf - -T "$manifest" 2>>"$archive_log" | ssh "${ssh_opts[@]}" "$ARCHIVE_USER@$ARCHIVE_HOST" "cat > '$archive_tar'"; then
          ssh "${ssh_opts[@]}" "$ARCHIVE_USER@$ARCHIVE_HOST" "cat > '$archive_manifest'" <"$manifest" || true
        else
          echo "archive_stream_failed" >>"$archive_log"
        fi
      else
        echo "archive_remote_dir_failed" >>"$archive_log"
      fi
    else
      echo "archive_key_missing" >>"$archive_log"
    fi
  fi
  while IFS= read -r file; do
    [[ -n "$file" ]] || continue
    rm -f -- "$file"
    dir="$(dirname "$file")"
    rmdir --ignore-fail-on-non-empty "$dir" 2>/dev/null || true
  done <"$manifest"
  find "$CHECKPOINT_ROOT" -depth -type d -empty -delete 2>/dev/null || true
else
  rm -f "$manifest"
fi

journalctl --vacuum-size="${JOURNAL_VACUUM_MB}M" >/dev/null 2>&1 || true
systemctl reset-failed logrotate.service >/dev/null 2>&1 || true
systemctl start logrotate.service >/dev/null 2>&1 || true
EOF

sudo chmod 0755 /usr/local/sbin/swarm-checkpoint-retention.sh

sudo tee /etc/systemd/system/swarm-checkpoint-retention.service >/dev/null <<'EOF'
[Unit]
Description=Prune excess SWARM MEE checkpoints on GPU core nodes
After=local-fs.target

[Service]
Type=oneshot
Environment=KEEP_COUNT=__KEEP_COUNT__
Environment=JOURNAL_VACUUM_MB=__JOURNAL_VACUUM_MB__
Environment=ARCHIVE_USER=__ARCHIVE_USER__
Environment=ARCHIVE_HOST=__ARCHIVE_HOST__
Environment=ARCHIVE_PATH=__ARCHIVE_PATH__
Environment=ARCHIVE_KEY_PATH=/home/Crypt/.ssh/google_compute_engine
ExecStart=/usr/local/sbin/swarm-checkpoint-retention.sh
EOF

sudo tee /etc/systemd/system/swarm-checkpoint-retention.timer >/dev/null <<'EOF'
[Unit]
Description=Run SWARM checkpoint retention periodically

[Timer]
OnBootSec=10m
OnUnitActiveSec=30m
Unit=swarm-checkpoint-retention.service

[Install]
WantedBy=timers.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now swarm-checkpoint-retention.timer
sudo systemctl start swarm-checkpoint-retention.service
sudo systemctl status swarm-checkpoint-retention.service --no-pager
df -h /
sudo systemctl status logrotate.service --no-pager || true
'@

$script = $script.Replace('__KEEP_COUNT__', $KeepCount.ToString()).
    Replace('__JOURNAL_VACUUM_MB__', $JournalVacuumMb.ToString()).
    Replace('__ARCHIVE_USER__', $ArchiveUser).
    Replace('__ARCHIVE_HOST__', $ArchiveHost).
    Replace('__ARCHIVE_PATH__', $ArchivePath)

gcloud compute ssh $Node --zone $Zone --tunnel-through-iap --command $script
