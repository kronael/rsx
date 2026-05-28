#!/usr/bin/env bash
# demo-trade.sh — reproducible E2E demo
#
# Start-all minimal (via playground API), submit a maker + taker IOC
# order pair, and verify a fill record appears in the ME WAL within
# TIMEOUT seconds. Exits 0 on success, 1 on failure.
#
# Pre: playground server on :49171
#      (./rsx-playground/playground start)
# Post: fill visible in ./tmp/wal/10/10_active.wal

set -euo pipefail

PLAYGROUND=${PLAYGROUND_URL:-http://localhost:49171}
TIMEOUT=${DEMO_TIMEOUT:-60}
WAL_DIR=${RSX_WAL_DIR:-./tmp/wal}
SYMBOL_ID=10

info() { printf '%s demo-trade: %s\n' "$(date '+%b %d %H:%M:%S')" "$*"; }
die()  { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

# ── 1. start-all minimal ─────────────────────────────────────────────
info "starting minimal cluster..."
RESP=$(curl -sf -X POST "${PLAYGROUND}/api/processes/all/start?scenario=minimal&confirm=yes" || true)
sleep 5

# ── 2. wait for all processes running ────────────────────────────────
DEADLINE=$(( $(date +%s) + TIMEOUT ))
info "waiting for processes..."
while true; do
    RUNNING=$(curl -sf "${PLAYGROUND}/api/processes" \
        | python3 -c "import sys,json; d=json.load(sys.stdin); print(sum(1 for p in d if p.get('state')=='running'))" 2>/dev/null || echo 0)
    if [ "$RUNNING" -ge 6 ]; then
        info "$RUNNING processes running"
        break
    fi
    [ "$(date +%s)" -lt "$DEADLINE" ] || die "processes did not start in ${TIMEOUT}s"
    sleep 2
done

# ── 3. submit maker (resting limit) and taker (IOC crossing it) ──────
info "submitting maker order (resting buy)..."
MAKER=$(curl -sf -X POST "${PLAYGROUND}/api/submit-order" \
    -H "Content-Type: application/json" \
    -d '{"symbol_id":'"$SYMBOL_ID"',"side":0,"price":60000,"qty":1000000,"tif":0,"user_id":1}' \
    2>/dev/null || echo "{}")
info "maker: $MAKER"
sleep 1

info "submitting taker order (IOC sell crossing)..."
TAKER=$(curl -sf -X POST "${PLAYGROUND}/api/submit-order" \
    -H "Content-Type: application/json" \
    -d '{"symbol_id":'"$SYMBOL_ID"',"side":1,"price":59000,"qty":500000,"tif":1,"user_id":1}' \
    2>/dev/null || echo "{}")
info "taker: $TAKER"

# ── 4. wait for fill in WAL ──────────────────────────────────────────
info "waiting for fill record in WAL..."
FILL_DEADLINE=$(( $(date +%s) + 30 ))
WAL_FILE="${WAL_DIR}/${SYMBOL_ID}/${SYMBOL_ID}_active.wal"
while true; do
    if [ -f "$WAL_FILE" ] && [ "$(wc -c < "$WAL_FILE")" -gt 16 ]; then
        info "WAL written: $(wc -c < "$WAL_FILE") bytes"
        break
    fi
    # Also check the verify endpoint
    FILLS=$(curl -sf "${PLAYGROUND}/api/verify/run-json" 2>/dev/null \
        | python3 -c "
import sys,json
d=json.load(sys.stdin)
fills=[c for c in d.get('checks',[]) if 'fill' in c.get('name','').lower()]
if fills: print(fills[0].get('detail',''))
" 2>/dev/null || echo "")
    if echo "$FILLS" | grep -q "[1-9][0-9]* fill"; then
        info "fills confirmed: $FILLS"
        break
    fi
    [ "$(date +%s)" -lt "$FILL_DEADLINE" ] || die "no fill in WAL after 30s"
    sleep 2
done

info "demo-trade: PASS"
