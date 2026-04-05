#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="${1:-/srv/swarm/repo}"
TARGET_ROOT="${2:-/srv/swarm/apps/mee}"
MANIFEST_SOURCE="${REPO_ROOT}/infra/profiles/mee_gpu_runtime_manifest.json"
SYNC_SCRIPT_SOURCE="${REPO_ROOT}/infra/provisioning/linux/swarm_mee_gpu_model_sync.sh"
PROMOTE_SCRIPT_SOURCE="${REPO_ROOT}/infra/provisioning/linux/swarm_mee_gpu_promote.py"
UNIT_SOURCE_DIR="${REPO_ROOT}/infra/systemd"
PATCH_SOURCE="${REPO_ROOT}/tmp/patches/mee_patch"
NODE_NAME="${SWARM_NODE_NAME:-$(hostname)}"

mkdir -p "$TARGET_ROOT" \
         "$TARGET_ROOT/model_research/experiments" \
         /srv/swarm/logs \
         /srv/swarm/gpu/reports \
         /srv/swarm/gpu/model-stage \
         /srv/swarm/apps/mee/deployed/coding.route/current \
         /srv/swarm/apps/mee/deployed/coding.verify/current \
         /srv/swarm/apps/mee/deployed/coding.dependency_map/current \
         /srv/swarm/apps/mee/deployed/coding.synthesize/current \
         /srv/swarm/apps/mee/deployed/voice.routing/current \
         /srv/swarm/apps/mee/deployed/voice.verification/current \
         /srv/swarm/apps/mee/deployed/crypto.signal_gate/current \
         /srv/swarm/apps/mee/deployed/crypto.risk_gate/current \
         /etc/swarm

if [[ ! -d "$PATCH_SOURCE" ]]; then
  echo "missing MEE patch source: $PATCH_SOURCE" >&2
  exit 2
fi

cp -f "$PATCH_SOURCE/mee_scheduler.py" "$TARGET_ROOT/mee_scheduler.py"
rm -rf "$TARGET_ROOT/model_research"
cp -R "$PATCH_SOURCE/model_research" "$TARGET_ROOT/model_research"

cp -f "$MANIFEST_SOURCE" /etc/swarm/mee-gpu-runtime-manifest.json
install -m 0755 "$SYNC_SCRIPT_SOURCE" /usr/local/bin/swarm-mee-gpu-model-sync
install -m 0755 "$PROMOTE_SCRIPT_SOURCE" /usr/local/bin/swarm-mee-gpu-promote
cp -f "$UNIT_SOURCE_DIR/mee-gpu-worker.service" /etc/systemd/system/mee-gpu-worker.service
cp -f "$UNIT_SOURCE_DIR/mee-gpu-model-sync.service" /etc/systemd/system/mee-gpu-model-sync.service
cp -f "$UNIT_SOURCE_DIR/mee-gpu-model-sync.timer" /etc/systemd/system/mee-gpu-model-sync.timer
cp -f "$UNIT_SOURCE_DIR/mee-gpu-promote.service" /etc/systemd/system/mee-gpu-promote.service
cp -f "$UNIT_SOURCE_DIR/mee-gpu-promote.timer" /etc/systemd/system/mee-gpu-promote.timer
chown -R crypticassassin:crypticassassin "$TARGET_ROOT"
chown -R crypticassassin:crypticassassin /srv/swarm/apps/mee/deployed
chown -R crypticassassin:crypticassassin /srv/swarm/gpu/reports /srv/swarm/gpu/model-stage /srv/swarm/logs

python3 - <<'PY'
import json
import os
from pathlib import Path

node_name = os.environ.get("SWARM_NODE_NAME") or os.uname().nodename
manifest = json.loads(Path("/etc/swarm/mee-gpu-runtime-manifest.json").read_text())
profile = (manifest.get("node_profiles") or {}).get(node_name) or {}
families = ",".join(profile.get("preferred_families") or [])
targets = ",".join(profile.get("preferred_targets") or [])
env_lines = [
    f"MEE_SEED={profile.get('seed', 52)}",
    f"MEE_POPULATION={profile.get('population', 20)}",
    f"MEE_ELITE={profile.get('elite', 4)}",
    f"MEE_CONCURRENT={profile.get('concurrent', 2)}",
    "MEE_GENERATIONS=9999",
    "MEE_DB_PATH=/srv/swarm/apps/mee/model_research/experiments/registry.db",
    "MEE_EXPORT_DIR=/srv/swarm/apps/mee/model_research/experiments",
    f"MEE_NODE_ROLE={profile.get('role', 'deploy_favored')}",
    f"MEE_PREFERRED_FAMILIES={families}",
    f"MEE_PREFERRED_TARGETS={targets}",
]
Path("/etc/swarm/mee-gpu.env").write_text("\n".join(env_lines) + "\n")
PY

systemctl daemon-reload
systemctl enable mee-gpu-model-sync.timer >/dev/null
systemctl restart mee-gpu-model-sync.service
systemctl restart mee-gpu-model-sync.timer
systemctl enable mee-gpu-promote.timer >/dev/null
systemctl restart mee-gpu-promote.service
systemctl restart mee-gpu-promote.timer

python3 - <<'PY'
import json
from pathlib import Path

required = [
    Path("/srv/swarm/apps/mee/mee_scheduler.py"),
    Path("/srv/swarm/apps/mee/model_research/engine.py"),
    Path("/srv/swarm/apps/mee/model_research/types.py"),
    Path("/srv/swarm/apps/mee/model_research/agents/model_generator.py"),
    Path("/srv/swarm/apps/mee/model_research/evaluation/evaluator.py"),
    Path("/srv/swarm/apps/mee/model_research/experiments/registry.py"),
]
missing = [str(path) for path in required if not path.exists()]
report = {
    "runtime_ready": not missing,
    "missing_required_files": missing,
}
Path("/srv/swarm/gpu/reports/mee-gpu-runtime.json").write_text(json.dumps(report, indent=2) + "\n")
PY

if python3 - <<'PY'
import json
from pathlib import Path
report = json.loads(Path("/srv/swarm/gpu/reports/mee-gpu-runtime.json").read_text())
raise SystemExit(0 if report.get("runtime_ready") else 1)
PY
then
  systemctl unmask mee-gpu-worker.service >/dev/null 2>&1 || true
  systemctl enable mee-gpu-worker.service >/dev/null
  systemctl restart mee-gpu-worker.service
else
  systemctl disable --now mee-gpu-worker.service >/dev/null 2>&1 || true
fi

echo "mee-gpu runtime installed"
echo "node=$NODE_NAME"
