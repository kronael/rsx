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
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CLI=${RSX_CLI:-${ROOT}/target/debug/rsx-cli}
WAL_ROOT=${RSX_WAL_ROOT:-${ROOT}/tmp/wal}
export DATABASE_URL=${DATABASE_URL:-postgres://rsx:rsx@127.0.0.1:5432/rsx}
export GATEWAY_URL=${GATEWAY_URL:-ws://127.0.0.1:8088}
export RSX_GW_JWT_SECRET=${RSX_GW_JWT_SECRET:-rsx-dev-secret-not-for-prod-padpad}

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
    "$CLI" wal file "$WAL_FILE" 2>/dev/null \
        | grep -c '"type":"FILL"' || true
}

submit() {
    local user=$1 side=$2 tif=$3 px=$4
    "$CLI" order place -u "$user" -s "$SYMBOL_ID" --side "$side" \
        --tif "$tif" --px "$px" --min-size --json
}

taker_account() {
    "$CLI" account show -u "$TAKER_USER" --json
}

best_bid() {
    "$CLI" md snapshot "$SYMBOL_ID" --json | python3 -c '
import json, sys
bbo = json.load(sys.stdin)["bbo"]
if not bbo or not bbo["bid_px"]:
    raise SystemExit(1)
print(bbo["bid_px"])
'
}

verify_trade() {
    local before=$1 before_positions=$2
    local positions book
    [[ $(fill_count) -gt $before ]] || return 1
    positions=$(taker_account) || return 1
    book=$(curl -sf "${PLAYGROUND}/api/book/${SYMBOL_ID}") || return 1
    python3 -c '
import json, sys
before, positions, book = (json.loads(value) for value in sys.argv[1:4])
symbol_id = int(sys.argv[4])
def net(account):
    return sum(
        int(row["net_qty"])
        for row in account["positions"] if int(row["symbol_id"]) == symbol_id
    )
ok = (
    net(positions) != net(before)
    and book.get("source") == "live"
)
raise SystemExit(0 if ok else 1)
' "$before_positions" "$positions" "$book" "$SYMBOL_ID"
}

verify_invariants() {
    "$CLI" verify --json "$WAL_ROOT"/*/ | python3 -c '
import json, sys
d = json.load(sys.stdin)
checks = d.get("checks", [])
required = (
    "Fills precede ORDER_DONE",
    "Position equals the sum of fills",
    "WAL seqs strictly increase",
    "BBO records are not crossed",
)
bad = [c for c in checks if c.get("status") == "fail"]
missing = [name for name in required if not any(
    name in c.get("name", "") and c.get("status") == "pass"
    for c in checks
)]
if d.get("status") == "fail" or bad or missing:
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
before_positions=$(taker_account) \
    || die "could not read taker position baseline"
crossing_px=$(best_bid) \
    || die "could not read a bid from live marketdata"
info "submitting taker user ${TAKER_USER} against live maker liquidity"
taker=$(submit "$TAKER_USER" sell ioc "$crossing_px") \
    || die "taker order failed: ${taker:-no outcome}"
python3 -c '
import json, sys
raise SystemExit(0 if json.loads(sys.argv[1])["status"] == "filled" else 1)
' "$taker" || die "client did not observe a fill: ${taker}"

wait_for "the trade in WAL, risk, and live marketdata" \
    verify_trade "$before" "$before_positions"
verify_invariants || die "Verify reported a failed or skipped required check"
logs_clean || die "fatal process log found"

trap - EXIT INT TERM
info "PASS: client, WAL, risk, marketdata, Verify, and logs agree"
