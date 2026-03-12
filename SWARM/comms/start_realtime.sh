#!/bin/bash
# start_realtime.sh — Start Jack's real-time voice agent with TLS tunnel
#
# This script:
# 1. Kills any existing voice agent / tunnel processes
# 2. Starts cloudflared tunnel (provides WSS for Twilio Media Streams)
# 3. Captures the tunnel URL
# 4. Starts voice_agent_realtime.py with the tunnel URL
#
# Usage:
#   ./start_realtime.sh              # Start server only
#   ./start_realtime.sh call sean    # Start + call sean

PORT=8091
COMMS_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR=/tmp

echo "=== Jack Voice Agent v2 (Realtime) ==="
echo ""

# Kill existing processes
echo "Stopping existing processes..."
pkill -f voice_agent_realtime 2>/dev/null
pkill -f "voice_agent.py" 2>/dev/null
pkill -f cloudflared 2>/dev/null
sleep 2

# Check cloudflared is installed
if ! command -v cloudflared &>/dev/null; then
    if [ -f /usr/local/bin/cloudflared ]; then
        CLOUDFLARED=/usr/local/bin/cloudflared
    elif [ -f "$COMMS_DIR/cloudflared" ]; then
        CLOUDFLARED="$COMMS_DIR/cloudflared"
    else
        echo "Installing cloudflared..."
        curl -sL -o /tmp/cloudflared https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64
        chmod +x /tmp/cloudflared
        sudo mv /tmp/cloudflared /usr/local/bin/cloudflared
        CLOUDFLARED=/usr/local/bin/cloudflared
    fi
else
    CLOUDFLARED=cloudflared
fi

echo "Starting cloudflared tunnel on port $PORT..."
$CLOUDFLARED tunnel --url http://localhost:$PORT > $LOG_DIR/cloudflared.log 2>&1 &
TUNNEL_PID=$!
echo "Tunnel PID: $TUNNEL_PID"

# Wait for tunnel URL to appear
echo "Waiting for tunnel URL..."
for i in $(seq 1 15); do
    TUNNEL_URL=$(grep -o 'https://[a-z0-9-]*\.trycloudflare\.com' $LOG_DIR/cloudflared.log 2>/dev/null | head -1)
    if [ -n "$TUNNEL_URL" ]; then
        break
    fi
    sleep 1
done

if [ -z "$TUNNEL_URL" ]; then
    echo "ERROR: Could not get tunnel URL after 15 seconds"
    echo "Check log: $LOG_DIR/cloudflared.log"
    cat $LOG_DIR/cloudflared.log
    exit 1
fi

echo ""
echo "Tunnel URL: $TUNNEL_URL"
echo ""

# Save tunnel URL for other scripts to read
echo "$TUNNEL_URL" > "$COMMS_DIR/.tunnel_url"

# Start voice agent
export VOICE_PUBLIC_URL="$TUNNEL_URL"
cd "$COMMS_DIR"
nohup python3 voice_agent_realtime.py > $LOG_DIR/voice_realtime.log 2>&1 &
AGENT_PID=$!
echo "Voice agent PID: $AGENT_PID"

sleep 2

# Health check
HEALTH=$(curl -s http://localhost:$PORT/health 2>/dev/null)
if echo "$HEALTH" | python3 -m json.tool 2>/dev/null; then
    echo ""
    echo "=== READY ==="
    echo "Server:    http://localhost:$PORT"
    echo "Tunnel:    $TUNNEL_URL"
    echo "Logs:      $LOG_DIR/voice_realtime.log"
    echo ""

    if [ "$1" = "call" ] && [ -n "$2" ]; then
        echo "Placing call to $2..."
        CONTEXT="${3:-}"
        python3 -c "
import os
os.environ['VOICE_PUBLIC_URL'] = '$TUNNEL_URL'
from voice_agent_realtime import place_call
place_call('$2', '$CONTEXT')
"
    else
        echo "To call:  VOICE_PUBLIC_URL=$TUNNEL_URL python3 voice_agent_realtime.py call sean"
        echo "To stop:  pkill -f voice_agent_realtime; pkill -f cloudflared"
    fi
else
    echo "ERROR: Voice agent health check failed"
    echo "Log output:"
    tail -20 $LOG_DIR/voice_realtime.log
    exit 1
fi
