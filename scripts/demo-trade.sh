#!/usr/bin/env bash
# demo-trade.sh -- prove one trade through the running RSX stack.

set -Eeuo pipefail

PLAYGROUND=${PLAYGROUND_URL:-http://localhost:49171}
TIMEOUT=${DEMO_TIMEOUT:-60}
START_CLUSTER=${DEMO_START_CLUSTER:-1}
SYMBOL_ID=${DEMO_SYMBOL_ID:-10}
SYMBOL_NAME=${DEMO_SYMBOL_NAME:-pengu}
WAL_FILE=${RSX_WAL_FILE:-./tmp/wal/${SYMBOL_NAME}/${SYMBOL_ID}/${SYMBOL_ID}_active.wal}
MAKER_USER=${DEMO_MAKER_USER:-1}
TAKER_USER=${DEMO_TAKER_USER:-2}

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
    curl -sf "${PLAYGROUND}/healthz" | python3 -c '
import json, sys
d = json.load(sys.stdin)
ok = (
    d.get("processes_total", 0) > 0
    and d.get("processes_running") == d.get("processes_total")
    and d.get("gateway") is True
    and d.get("marketdata") is True
)
raise SystemExit(0 if ok else 1)
' 2>/dev/null
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
    and status.get("levels", 0) > 0
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

verify_trade() {
    local before=$1 before_user_fills=$2
    local fills positions book
    [[ $(fill_count) -gt $before ]] || return 1
    fills=$(curl -sf -H "x-user-id: ${TAKER_USER}" \
        "${PLAYGROUND}/v1/fills?user_id=${TAKER_USER}&sym=${SYMBOL_ID}&limit=10") || return 1
    positions=$(curl -sf "${PLAYGROUND}/api/risk/users/${TAKER_USER}") || return 1
    book=$(curl -sf "${PLAYGROUND}/api/book/${SYMBOL_ID}") || return 1
    python3 -c '
import json, sys
fills, positions, book = (json.loads(value) for value in sys.argv[1:4])
if not isinstance(positions, list):
    raise SystemExit(1)
position_seen = any(
    int(row.get("symbol_id", -1)) == int(sys.argv[4])
    and int(row.get("long_qty", 0)) != int(row.get("short_qty", 0))
    for row in positions
)
ok = (
    len(fills) > int(sys.argv[5])
    and position_seen
    and book.get("source") == "live"
)
raise SystemExit(0 if ok else 1)
' "$fills" "$positions" "$book" "$SYMBOL_ID" "$before_user_fills"
}

verify_invariants() {
    curl -sf -X POST "${PLAYGROUND}/api/verify/run-json" | python3 -c '
import json, sys
d = json.load(sys.stdin)
checks = d.get("checks", [])
required = (
    "RSX processes running",
    "Fills precede ORDER_DONE",
    "Exactly-one completion",
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

curl -sf "${PLAYGROUND}/api/processes" >/dev/null \
    || die "playground is not running at ${PLAYGROUND}"
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
    info "starting the market maker"
    curl -sf -X POST "${PLAYGROUND}/api/maker/start?confirm=yes" \
        -H 'x-confirm: yes' >/dev/null || true
    if [[ -z ${initial[market-maker]+x} ]]
    then
        started=true
    fi
fi
wait_for "the full declared process set" cluster_ready
wait_for "real maker liquidity" maker_liquid

before=$(fill_count)
before_user_fills=$(curl -sf -H "x-user-id: ${TAKER_USER}" \
    "${PLAYGROUND}/v1/fills?user_id=${TAKER_USER}&sym=${SYMBOL_ID}&limit=1000" \
    | python3 -c 'import json, sys; print(len(json.load(sys.stdin)))')
info "submitting maker user ${MAKER_USER}"
maker=$(submit "$MAKER_USER" buy GTC 0.050000 10) \
    || die "maker order request failed"
[[ $maker != *error* && $maker != *rejected* ]] \
    || die "maker order was not accepted: ${maker}"

info "submitting taker user ${TAKER_USER}"
taker=$(submit "$TAKER_USER" sell IOC 0.049000 10) \
    || die "taker order request failed"
[[ $taker == *filled* ]] || die "client did not observe a fill: ${taker}"

wait_for "the trade in WAL, risk, and live marketdata" \
    verify_trade "$before" "$before_user_fills"
verify_invariants || die "Verify reported a failed or skipped required check"
logs_clean || die "fatal process log found"

trap - EXIT INT TERM
info "PASS: client, WAL, risk, marketdata, Verify, and logs agree"
