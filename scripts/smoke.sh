#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${RSX_URL:-http://localhost:8080}"
TIMEOUT=10

# 1. Assert server running
if ! curl -sf "$BASE_URL/api/processes" > /dev/null; then
    echo "smoke: server not running at $BASE_URL" >&2
    exit 1
fi
echo "smoke: server up"

# 2. Wait for maker running
echo "smoke: waiting for maker..."
deadline=$(( $(date +%s) + TIMEOUT ))
while true; do
    status=$(curl -sf "$BASE_URL/api/maker/status" 2>/dev/null \
        | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('status',''))" \
        2>/dev/null || true)
    if [ "$status" = "running" ]; then
        echo "smoke: maker running"
        break
    fi
    if [ "$(date +%s)" -ge "$deadline" ]; then
        echo "smoke: timeout waiting for maker" >&2
        exit 1
    fi
    sleep 1
done

# 3. Place one limit order
ORDER=$(curl -sf -X POST "$BASE_URL/api/orders" \
    -H "Content-Type: application/json" \
    -d '{"symbol":"BTC-PERP","side":"buy","price":50000,"qty":1}')
echo "smoke: order placed: $ORDER"

# 4. Poll for fill within 10s
echo "smoke: waiting for fill..."
deadline=$(( $(date +%s) + TIMEOUT ))
while true; do
    fills=$(curl -sf "$BASE_URL/x/live-fills" 2>/dev/null || true)
    if echo "$fills" | python3 -c "import sys,json; d=json.load(sys.stdin); sys.exit(0 if d else 1)" 2>/dev/null; then
        echo "smoke: fill received"
        exit 0
    fi
    if [ "$(date +%s)" -ge "$deadline" ]; then
        echo "smoke: timeout waiting for fill" >&2
        exit 1
    fi
    sleep 1
done
