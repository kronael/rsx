#!/usr/bin/env bash
# demo-trade.sh — reproducible E2E demo
#
# Start-all minimal (via playground API), submit a maker + taker IOC
# order pair through the real gateway path, and verify a FILL record
# lands in the ME WAL within TIMEOUT seconds. Exits 0 on success.
#
# Pre: playground server on :49171
#      (./rsx-playground/playground start)
# Post: FILL visible in ./tmp/wal/pengu/10/10_active.wal
#
# Units are HUMAN (the /api/orders/test form converts to raw):
#   PENGU (id 10): price_dec 6, qty_dec 4, lot 100000 → qty must be a
#   multiple of 10 (10^5 lot / 10^4 qty-scale). price 0.05, qty 10.

set -euo pipefail

PLAYGROUND=${PLAYGROUND_URL:-http://localhost:49171}
TIMEOUT=${DEMO_TIMEOUT:-60}
SYMBOL_ID=10
SYMBOL_NAME=${SYMBOL_NAME:-pengu}
WAL_FILE=${RSX_WAL_FILE:-./tmp/wal/${SYMBOL_NAME}/${SYMBOL_ID}/${SYMBOL_ID}_active.wal}

info() { printf '%s demo-trade: %s\n' "$(date '+%b %d %H:%M:%S')" "$*"; }
die()  { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

# ── 1. start-all minimal ─────────────────────────────────────────────
info "starting minimal cluster..."
curl -sf -X POST \
    "${PLAYGROUND}/api/processes/all/start?scenario=minimal" \
    -H 'x-confirm: yes' >/dev/null || true

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

# Risk promotes to Live only after warm-catchup from the ME's TLS
# replication stream. Give it a moment so the first order isn't
# rejected while risk is still catching up.
sleep 3

submit() {  # side tif price qty
    curl -sf -X POST "${PLAYGROUND}/api/orders/test" \
        -d "symbol_id=${SYMBOL_ID}&order_type=limit&user_id=1&side=$1&tif=$2&price=$3&qty=$4" \
        2>/dev/null || echo "(no response)"
}

# ── 3. submit maker (resting buy) then taker (IOC sell crossing) ─────
info "submitting maker (resting buy 0.05 x 10)..."
MAKER=$(submit buy GTC 0.050000 10)
info "maker: $MAKER"
sleep 1

info "submitting taker (IOC sell 0.049 x 10, crosses)..."
TAKER=$(submit sell IOC 0.049000 10)
info "taker: $TAKER"

# ── 4. verify a FILL record in the ME WAL ────────────────────────────
info "checking ME WAL for a FILL record..."
FILL_DEADLINE=$(( $(date +%s) + 30 ))
while true; do
    if [ -f "$WAL_FILE" ]; then
        FILLS=$(cargo run -q --bin rsx-cli -- dump "$WAL_FILE" 2>/dev/null \
            | grep -c '"type":"FILL"' || true)
        if [ "${FILLS:-0}" -gt 0 ]; then
            info "FILL records in WAL: $FILLS"
            break
        fi
    fi
    [ "$(date +%s)" -lt "$FILL_DEADLINE" ] || die "no FILL in WAL after 30s (path: $WAL_FILE)"
    sleep 2
done

info "demo-trade: PASS"
