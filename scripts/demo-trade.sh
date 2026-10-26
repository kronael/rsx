#!/usr/bin/env bash
# demo-trade.sh -- prove one trade through the running RSX stack.

set -Eeuo pipefail

PLAYGROUND=${PLAYGROUND_URL:-http://localhost:49171}
TIMEOUT=${DEMO_TIMEOUT:-60}
START_CLUSTER=${DEMO_START_CLUSTER:-1}
SYMBOL_ID=${DEMO_SYMBOL_ID:-10}
SYMBOL_NAME=${DEMO_SYMBOL_NAME:-pengu}
WAL_FILE=${RSX_WAL_FILE:-./tmp/wal/${SYMBOL_NAME}/${SYMBOL_ID}/${SYMBOL_ID}_active.wal}
TAKER_USER=${DEMO_TAKER_USER:-2}
PLAYGROUND_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../rsx-playground" && pwd)

started=false
declare -A initial=()
log_baseline=0

info() { printf '%s INFO demo-trade: %s\n' "$(date '+%b %d %H:%M:%S')" "$*"; }
die() { printf 'Failed: demo-trade: %s\n' "$*" >&2; exit 1; }

process_names() {
    curl -sf "${PLAYGROUND}/api/processes" | python3 -c '
import json, sys
for process in json.load(sys.stdin):
    if process.get("state") == "running":
        print(process["name"])
'
}

cleanup() {
    local status=$?
    if [[ $status -ne 0 ]] && $started
    then
        info "cleaning up processes started by this run"
        while IFS= read -r name
        do
            if [[ -z ${initial[$name]+x} ]]
            then
                curl -sf -X POST \
                    "${PLAYGROUND}/api/processes/${name}/stop" >/dev/null || true
            fi
        done < <(process_names 2>/dev/null || true)
    fi
    exit "$status"
}
trap cleanup EXIT INT TERM

wait_for() {
    local description=$1
    shift
    local deadline=$(( $(date +%s) + TIMEOUT ))
    until "$@"
    do
        if [[ $(date +%s) -ge $deadline ]]
        then
            die "timed out waiting for ${description}"
        fi
        sleep 1
    done
}

cluster_ready() {
    local health processes
    health=$(curl -sf "${PLAYGROUND}/healthz") || return 1
    processes=$(curl -sf "${PLAYGROUND}/api/processes") || return 1
    curl -sf http://127.0.0.1:9900/ready >/dev/null || return 1
    python3 -c '
import json, sys
health = json.loads(sys.argv[1])
processes = json.loads(sys.argv[2])
ok = (
    bool(processes)
    and all(process.get("state") == "running" for process in processes)
    and health.get("postgres") is True
    and health.get("gateway") is True
    and health.get("marketdata") is True
)
raise SystemExit(0 if ok else 1)
' "$health" "$processes" 2>/dev/null
}

maker_liquid() {
    local status book
    status=$(curl -sf "${PLAYGROUND}/api/maker/status") || return 1
    book=$(curl -sf "${PLAYGROUND}/api/book/${SYMBOL_ID}") || return 1
    python3 -c '
import json, sys
status, book = (json.loads(value) for value in sys.argv[1:])
ok = (
    status.get("running") is True
    and not status.get("errors")
    and book.get("source") in {"live", "wal"}
    and bool(book.get("bids"))
    and bool(book.get("asks"))
)
raise SystemExit(0 if ok else 1)
' "$status" "$book"
}

fill_count() {
    if [[ ! -f $WAL_FILE ]]
    then
        printf '0\n'
        return
    fi
    cargo run -q --bin rsx-cli -- dump "$WAL_FILE" 2>/dev/null \
        | grep -c '"type":"FILL"' || true
}

submit() {
    local user=$1 side=$2 tif=$3 price=$4 qty=$5
    curl -sf -X POST "${PLAYGROUND}/api/orders/test" \
        -H 'content-type: application/x-www-form-urlencoded' \
        -d "symbol_id=${SYMBOL_ID}&order_type=limit&user_id=${user}&side=${side}&tif=${tif}&price=${price}&qty=${qty}"
}

crossing_order() {
    local book symbols
    book=$(curl -sf "${PLAYGROUND}/api/book/${SYMBOL_ID}") || return 1
    symbols=$(curl -sf "${PLAYGROUND}/v1/symbols") || return 1
    python3 -c '
import json, sys
sys.path.insert(0, sys.argv[4])
import notional
book, symbols = (json.loads(value) for value in sys.argv[1:3])
meta = next(row for row in symbols["symbols"] if int(row["id"]) == int(sys.argv[3]))
bid = int(book["bids"][0]["px"])
price_decimals = int(meta["price_decimals"])
qty_decimals = int(meta["qty_decimals"])
lot_size = int(meta["lot_size"])
# Risk rejects an order below the symbol minimum notional: sell the
# smallest lot multiple that reaches it at the bid.
qty = notional.size_qty(
    lot_size, bid, notional.catalog_min_notional(meta), lot_size)
print(f"{bid / (10 ** price_decimals):.{price_decimals}f} {qty / (10 ** qty_decimals):.{qty_decimals}f}")
' "$book" "$symbols" "$SYMBOL_ID" "$PLAYGROUND_DIR"
}

verify_trade() {
    local before=$1 before_positions=$2
    local positions book
    [[ $(fill_count) -gt $before ]] || return 1
    positions=$(curl -sf "${PLAYGROUND}/api/risk/users/${TAKER_USER}") || return 1
    book=$(curl -sf "${PLAYGROUND}/api/book/${SYMBOL_ID}") || return 1
    python3 -c '
import json, sys
before, positions, book = (json.loads(value) for value in sys.argv[1:4])
if not isinstance(before, list) or not isinstance(positions, list):
    raise SystemExit(1)
symbol_id = int(sys.argv[4])
def net(rows):
    return sum(
        int(row.get("long_qty", 0)) - int(row.get("short_qty", 0))
        for row in rows if int(row.get("symbol_id", -1)) == symbol_id
    )
ok = (
    net(positions) != net(before)
    and book.get("source") == "live"
)
raise SystemExit(0 if ok else 1)
' "$before_positions" "$positions" "$book" "$SYMBOL_ID"
}

verify_invariants() {
    curl -sf -X POST "${PLAYGROUND}/api/verify/run-json" | python3 -c '
import json, sys
d = json.load(sys.stdin)
checks = d.get("checks", [])
required = (
    "RSX processes running",
    "Fills precede ORDER_DONE",
    "Position = sum of fills",
    "Tips monotonic",
    "No crossed book",
)
bad = [c for c in checks if c.get("status") == "fail"]
missing = [name for name in required if not any(
    name in c.get("name", "") and c.get("status") == "pass"
    for c in checks
)]
if not d.get("ready") or bad or missing:
    print(json.dumps({"failed": bad, "missing_or_skipped": missing}), file=sys.stderr)
    raise SystemExit(1)
' 2>/dev/null
}

logs_clean() {
    curl -sf "${PLAYGROUND}/api/logs?limit=2000" | python3 -c '
import json, re, sys
lines = json.load(sys.stdin).get("lines", [])
fatal = re.compile(r"panicked at|\bfatal\b|missing required (configuration|config)", re.I)
bad = [line for line in lines if fatal.search(line)]
baseline = int(sys.argv[1])
if len(bad) > baseline:
    print("\n".join(bad[-20:]), file=sys.stderr)
    raise SystemExit(1)
' "$log_baseline" 2>/dev/null
}

wait_for "Playground at ${PLAYGROUND}" curl -sf "${PLAYGROUND}/api/processes" >/dev/null
while IFS= read -r name
do
    initial[$name]=1
done < <(process_names)
log_baseline=$(curl -sf "${PLAYGROUND}/api/logs?limit=2000" | python3 -c '
import json, re, sys
fatal = re.compile(r"panicked at|\bfatal\b|missing required (configuration|config)", re.I)
print(sum(bool(fatal.search(line)) for line in json.load(sys.stdin).get("lines", [])))
')

if [[ $START_CLUSTER == 1 ]]
then
    info "starting the minimal cluster"
    curl -sf -X POST \
        "${PLAYGROUND}/api/processes/all/start?scenario=minimal" \
        -H 'x-confirm: yes' >/dev/null \
        || die "minimal cluster failed to start"
    started=true
fi

if [[ $START_CLUSTER == 1 ]]
then
    wait_for "the full declared process set" cluster_ready
    info "starting the market maker"
    curl -sf -X POST "${PLAYGROUND}/api/maker/start?confirm=yes" \
        -H 'x-confirm: yes' >/dev/null || true
    if [[ -z ${initial[market-maker]+x} ]]
    then
        started=true
    fi
fi
if [[ $START_CLUSTER != 1 ]]
then
    wait_for "the full declared process set" cluster_ready
fi
wait_for "real maker liquidity" maker_liquid

before=$(fill_count)
before_positions=$(curl -sf "${PLAYGROUND}/api/risk/users/${TAKER_USER}") \
    || die "could not read taker position baseline"
read -r crossing_price crossing_qty < <(crossing_order) \
    || die "could not derive a crossing order from live liquidity"
info "submitting taker user ${TAKER_USER} against live maker liquidity"
taker=$(submit "$TAKER_USER" sell IOC "$crossing_price" "$crossing_qty") \
    || die "taker order request failed"
[[ $taker == *filled* ]] || die "client did not observe a fill: ${taker}"

wait_for "the trade in WAL, risk, and live marketdata" \
    verify_trade "$before" "$before_positions"
verify_invariants || die "Verify reported a failed or skipped required check"
logs_clean || die "fatal process log found"

trap - EXIT INT TERM
info "PASS: client, WAL, risk, marketdata, Verify, and logs agree"
