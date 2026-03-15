#!/bin/bash
# deploy-signal-ingress.sh
# Deploy and start the BUNNY Signal ingress daemon on swarm-mainframe.
#
# Usage:
#   ./deploy/deploy-signal-ingress.sh            # full deploy
#   ./deploy/deploy-signal-ingress.sh --restart  # restart service only
#   ./deploy/deploy-signal-ingress.sh --status   # show service + directive status
#   ./deploy/deploy-signal-ingress.sh --logs      # tail live logs
#
# Requires: git, python3, signal-cli, systemd

set -euo pipefail

REPO_DIR="/opt/repos/BUNNY"
SWARM_DIR="$REPO_DIR/SWARM"
COMMS_DIR="$SWARM_DIR/comms"
SERVICE_NAME="signal-ingress"
SERVICE_FILE="$COMMS_DIR/signal-ingress.service"
SYSTEMD_PATH="/etc/systemd/system/$SERVICE_NAME.service"
DATA_DIR="/opt/swarm/data"
LOG_TAG="signal-ingress"

echo "=== BUNNY Signal Ingress Deployer ==="
echo ""

# ── Helpers ──────────────────────────────────────────────────────────────────

check_root() {
    if [ "$EUID" -ne 0 ]; then
        echo "ERROR: This script must be run as root (sudo)."
        exit 1
    fi
}

# ── Modes ─────────────────────────────────────────────────────────────────────

if [ "${1:-}" = "--status" ]; then
    echo "── Service status ──"
    systemctl status $SERVICE_NAME --no-pager || true
    echo ""
    echo "── Directive store ──"
    cd "$REPO_DIR"
    PYTHONPATH="$REPO_DIR" python3 -m SWARM.comms.signal_ingress --status
    exit 0
fi

if [ "${1:-}" = "--logs" ]; then
    journalctl -u $SERVICE_NAME -f --no-hostname
    exit 0
fi

if [ "${1:-}" = "--restart" ]; then
    check_root
    systemctl restart $SERVICE_NAME
    echo "Restarted $SERVICE_NAME"
    systemctl status $SERVICE_NAME --no-pager
    exit 0
fi

# ── Full deploy ───────────────────────────────────────────────────────────────

check_root

# 1. Pull latest code
echo "[1/7] Pulling latest from GitHub..."
cd "$REPO_DIR"
git pull origin main
echo "      HEAD: $(git rev-parse --short HEAD)"

# 2. Create data directory
echo "[2/7] Creating data directory $DATA_DIR..."
mkdir -p "$DATA_DIR"
chown -R bunny:bunny "$DATA_DIR" 2>/dev/null || true

# 3. Check signal-cli installed
echo "[3/7] Checking signal-cli..."
if ! command -v signal-cli &>/dev/null; then
    echo "      signal-cli not found at PATH — checking /usr/local/bin/signal-cli..."
    if [ ! -f /usr/local/bin/signal-cli ]; then
        echo "ERROR: signal-cli not installed."
        echo "       Install via: https://github.com/AsamK/signal-cli/releases"
        echo "       Then re-run this script."
        exit 1
    fi
fi
SIGNAL_VER=$(signal-cli --version 2>/dev/null || /usr/local/bin/signal-cli --version 2>/dev/null || echo "unknown")
echo "      signal-cli: $SIGNAL_VER"

# 4. Verify .env has real Signal credentials
echo "[4/7] Verifying .env credentials..."
if grep -q "CHANGEME" "$COMMS_DIR/.env"; then
    echo "WARNING: .env still has CHANGEME values — some channels may not work."
fi
SIGNAL_NUM=$(grep SIGNAL_SENDER_NUMBER "$COMMS_DIR/.env" | cut -d= -f2)
if [ "$SIGNAL_NUM" = "+1CHANGEME" ] || [ -z "$SIGNAL_NUM" ]; then
    echo "ERROR: SIGNAL_SENDER_NUMBER not set in $COMMS_DIR/.env"
    exit 1
fi
echo "      Signal sender: $SIGNAL_NUM"

# 5. Verify signal-cli account is registered
echo "[5/7] Checking signal-cli account registration..."
REGISTERED=$(signal-cli --config /home/bunny/.local/share/signal-cli listAccounts 2>/dev/null \
             || signal-cli listAccounts 2>/dev/null \
             || echo "")
if echo "$REGISTERED" | grep -q "$SIGNAL_NUM"; then
    echo "      Account $SIGNAL_NUM is registered."
else
    echo "WARNING: $SIGNAL_NUM not found in signal-cli accounts."
    echo "         Run: signal-cli -u $SIGNAL_NUM register"
    echo "         Then: signal-cli -u $SIGNAL_NUM verify <code>"
fi

# 6. Install systemd service
echo "[6/7] Installing systemd service..."
cp "$SERVICE_FILE" "$SYSTEMD_PATH"

# Create bunny user if needed
if ! id -u bunny &>/dev/null; then
    echo "      Creating bunny system user..."
    useradd --system --no-create-home --shell /usr/sbin/nologin bunny
    # Give bunny read access to repos
    chown -R bunny:bunny "$REPO_DIR" 2>/dev/null || true
fi

# Give bunny git push access (needs SSH key or HTTPS credential)
if [ -f /home/bunny/.gitconfig ]; then
    true
else
    mkdir -p /home/bunny
    cat > /home/bunny/.gitconfig <<EOF
[user]
    name = Swarm Mainframe
    email = swarm@calculusholdings.com
EOF
    chown -R bunny:bunny /home/bunny
fi

systemctl daemon-reload
systemctl enable $SERVICE_NAME
systemctl restart $SERVICE_NAME

# 7. Verify
echo "[7/7] Verifying startup..."
sleep 3
if systemctl is-active --quiet $SERVICE_NAME; then
    echo ""
    echo "=== SIGNAL INGRESS ONLINE ==="
    echo "  Service:  $SERVICE_NAME (enabled, running)"
    echo "  Number:   $SIGNAL_NUM"
    echo "  DB:       $DATA_DIR/directives.db"
    echo "  Logs:     journalctl -u $SERVICE_NAME -f"
    echo ""
    echo "Commands:"
    echo "  Status:   $0 --status"
    echo "  Logs:     $0 --logs"
    echo "  Restart:  sudo $0 --restart"
else
    echo "ERROR: Service failed to start."
    journalctl -u $SERVICE_NAME --no-pager -n 30
    exit 1
fi
