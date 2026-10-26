#!/usr/bin/env bash
# Publish sustained external WebSocket latency using the existing stress client.
set -euo pipefail
cd "$(dirname "$0")/.."

BASELINE=${BASELINE:-bench-e2e-latest.json}
REPORT_DIR=${REPORT_DIR:-rsx-playground/tmp/bench}
RATE_CONFIGURED=${RATE+x}
RATE=${RATE:-1000}
MODE=${MODE:-single}
RATES=${RATES:-"1000 5000 10000 25000 50000 100000"}
LONG_DURATION=${LONG_DURATION:-600}
REPEATS=${REPEATS:-3}
MAX_P99_SPREAD=${MAX_P99_SPREAD:-0.10}
N=${N:-2000}
DURATION=${DURATION:-$(( (N + RATE - 1) / RATE ))}
MIN_SAMPLES=${MIN_SAMPLES:-$N}
MIN_RATIO=${MIN_RATIO:-0.95}
USERS_CONFIGURED=${USERS+x}
USERS=${USERS:-10}
CONNECTIONS=${CONNECTIONS:-$USERS}
PROCESSES=${PROCESSES:-20}
CONNECTION_STEPS=${CONNECTION_STEPS:-"1 10 100 1000"}
GW_URL=${RSX_STRESS_GW_URL:-ws://127.0.0.1:8088}
JWT_SECRET=${RSX_GW_JWT_SECRET:-}
SYMBOLS=${RSX_STRESS_SYMBOLS:-10}
if [ "$SYMBOLS" = 10,3,1 ]; then DEFAULT_SCENARIO=trio; else DEFAULT_SCENARIO=minimal; fi
SCENARIO=${RSX_STRESS_SCENARIO:-$DEFAULT_SCENARIO}
export RSX_STRESS_SCENARIO="$SCENARIO"
LOT_SIZE=${RSX_STRESS_LOT_SIZE:-100000}
VALIDATE=""
[ "${1:-}" != "--validate-report" ] || { [ "$#" -eq 2 ] || exit 2; VALIDATE=$2; }
[ "${1:-}" != "--validate-mode" ] || { [ "$#" -eq 1 ] || exit 2; MODE_VALIDATE=1; }
[ "$#" -eq 0 ] || [ -n "$VALIDATE" ] || [ "${MODE_VALIDATE:-}" = 1 ] || { echo "unknown argument: $1" >&2; exit 2; }
mkdir -p "$REPORT_DIR"

validate_mode() {
 case "$MODE" in single|staircase|connections|long) ;; *) echo "invalid MODE=$MODE (expected single, staircase, connections, or long)" >&2; return 2;; esac
 [[ "$RATE" =~ ^[1-9][0-9]*$ ]] || { echo 'RATE must be a positive integer' >&2; return 2; }
 [[ "$LONG_DURATION" =~ ^[1-9][0-9]*$ ]] || { echo 'LONG_DURATION must be a positive integer' >&2; return 2; }
 [[ "$REPEATS" =~ ^[1-9][0-9]*$ ]] || { echo 'REPEATS must be a positive integer' >&2; return 2; }
 [[ "$USERS" =~ ^[1-9][0-9]*$ ]] || { echo 'USERS must be a positive integer' >&2; return 2; }
 [[ "$CONNECTIONS" =~ ^[1-9][0-9]*$ ]] || { echo 'CONNECTIONS must be a positive integer' >&2; return 2; }
 [[ "$PROCESSES" =~ ^[1-9][0-9]*$ ]] || { echo 'PROCESSES must be a positive integer' >&2; return 2; }
 [[ "$SYMBOLS" =~ ^[0-9]+(,[0-9]+)*$ ]] || { echo 'RSX_STRESS_SYMBOLS must be comma-separated integers' >&2; return 2; }
 [[ "$LOT_SIZE" =~ ^[1-9][0-9]*$ ]] || { echo 'RSX_STRESS_LOT_SIZE must be a positive integer' >&2; return 2; }
 python3 - "$MAX_P99_SPREAD" <<'PY'
import sys
try: value=float(sys.argv[1])
except ValueError: raise SystemExit('MAX_P99_SPREAD must be numeric')
if not 0 <= value < 1: raise SystemExit('MAX_P99_SPREAD must be in [0,1)')
PY
 if [ "$MODE" = staircase ]; then
  for rate in $RATES; do [[ "$rate" =~ ^[1-9][0-9]*$ ]] || { echo "invalid staircase rate: $rate" >&2; return 2; }; done
  [ -n "$RATES" ] || { echo 'RATES must not be empty' >&2; return 2; }
 fi
 if [ "$MODE" = connections ]; then
  for connections in $CONNECTION_STEPS
  do
   [[ "$connections" =~ ^[1-9][0-9]*$ ]] || { echo "invalid connection step: $connections" >&2; return 2; }
  done
  [[ -n $CONNECTION_STEPS ]] || { echo 'CONNECTION_STEPS must not be empty' >&2; return 2; }
  for rate in $RATES
  do
   [[ "$rate" =~ ^[1-9][0-9]*$ ]] || { echo "invalid staircase rate: $rate" >&2; return 2; }
  done
 fi
}

validate() {
python3 - "$1" "$2" "$3" <<'PY'
import json, math, sys
path, need, ratio = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
try: d=json.load(open(path)); m=d['metrics']; lat=d['latency_us']
except (OSError,json.JSONDecodeError,KeyError,TypeError) as e:
 print(f'invalid measurement: malformed report: {e}',file=sys.stderr); raise SystemExit(1)
bad=[]
def count(k, src=m):
 v=src.get(k)
 if not isinstance(v,int) or isinstance(v,bool) or v<0: bad.append(f'invalid {k}'); return 0
 return v
offered,submitted=count('offered'),count('submitted'); accepted,rejected=count('accepted'),count('rejected')
completed,timed_out=count('completed'),count('timed_out'); pending,errors=count('pending'),count('errors')
send_errors=count('send_errors'); samples=count('samples',lat)
if offered != submitted+send_errors: bad.append('offered accounting is open')
if submitted != completed+timed_out+pending: bad.append('order accounting is open')
if completed != accepted+rejected+errors: bad.append('response accounting is open')
if pending: bad.append('pending outcomes remain')
terminal=(completed+timed_out)/submitted if submitted else 0
achieved=m.get('achieved_rate'); target=d.get('config',{}).get('target_rate')
throughput=achieved/target if isinstance(achieved,(int,float)) and target else 0
if terminal < ratio: bad.append('terminal ratio below threshold')
if throughput < ratio: bad.append('achieved/offered ratio below threshold')
if accepted<=0 or samples<=0: bad.append('zero accepted latency samples')
if samples != accepted: bad.append('samples do not equal accepted')
if samples < need: bad.append(f'insufficient samples ({samples} < {need})')
timestamp_errors=m.get('timestamp_errors',{})
if not isinstance(timestamp_errors,dict) or any(timestamp_errors.values()): bad.append('invalid server timestamps')
server=d.get('server_latency_ns',{})
filled=count('filled_orders')
stable={
 'internal':accepted,
 'gateway_to_risk':accepted,
 'risk_to_me':filled,
 'matching':filled,
 'match_done_to_gateway':accepted,
}
for leg, expected in stable.items():
 row=server.get(leg,{})
 leg_samples=row.get('samples')
 if leg_samples != expected: bad.append(f'{leg} samples do not equal {expected}')
 if leg_samples is None or leg_samples < need: bad.append(f'insufficient {leg} samples')
 for percentile in ('p50','p99'):
  value=row.get(percentile)
  if not isinstance(value,(int,float)) or isinstance(value,bool) or not math.isfinite(value) or value<0:
   bad.append(f'{leg} {percentile} unavailable')
for k in ('p50','p95','p99','p99_9','max'):
 v=lat.get(k)
 if not isinstance(v,(int,float)) or isinstance(v,bool) or not math.isfinite(v) or v<0: bad.append(f'{k} unavailable')
nonperf=[x for x in d.get('failures',[]) if not str(x).startswith('p99=')]
if nonperf: bad.append('stress correctness failed: '+'; '.join(nonperf))
symbols=d.get('config', {}).get('symbols', [])
metric_names=('offered','submitted','accepted','rejected','completed','timed_out','pending','errors','send_errors','fill_attempts','filled_orders','fills')
for symbol in symbols:
 per=d.get('per_symbol',{}).get(str(symbol),{})
 pm=per.get('metrics',{})
 if pm.get('offered',0)<=0: bad.append(f'symbol {symbol} received no work')
 if pm.get('completed',0)<=0: bad.append(f'symbol {symbol} completed no work')
 if per.get('latency_us',{}).get('samples',0)<=0: bad.append(f'symbol {symbol} has no latency samples')
 for leg, expected_name in (('internal','accepted'),('gateway_to_risk','accepted'),('risk_to_me','filled_orders'),('matching','filled_orders'),('match_done_to_gateway','accepted')):
  if per.get('server_latency_ns',{}).get(leg,{}).get('samples') != pm.get(expected_name,0):
   bad.append(f'symbol {symbol} {leg} samples do not close')
for name in metric_names:
 if symbols and sum(d.get('per_symbol',{}).get(str(symbol),{}).get('metrics',{}).get(name,0) for symbol in symbols) != m.get(name,0):
  bad.append(f'per-symbol {name} accounting is open')
if bad: print('invalid measurement: '+'; '.join(dict.fromkeys(bad)),file=sys.stderr); raise SystemExit(1)
print(json.dumps({'valid':True,'terminal_ratio':terminal,'throughput_ratio':throughput,'report':d}))
PY
}

validate_mode
if [ "${MODE_VALIDATE:-}" = 1 ]; then exit; fi
if [ -n "$VALIDATE" ]; then validate "$VALIDATE" "$MIN_SAMPLES" "$MIN_RATIO" >/dev/null; exit; fi

if [ -z "$JWT_SECRET" ]; then
 case "$GW_URL" in
  ws://127.0.0.1:*|ws://localhost:*) JWT_SECRET=rsx-dev-secret-not-for-prod-padpad ;;
  *) echo 'RSX_GW_JWT_SECRET must be set for a non-local gateway' >&2; exit 2 ;;
 esac
fi

curl -fsS -m 2 http://127.0.0.1:49171/healthz >/dev/null || { echo '[latency-publish] playground unhealthy' >&2; exit 2; }
seed_crossing_book() {
 local run_rate=$1 run_duration=$2 orders side_orders refresh_ms symbol_id lots_per_order=1 attempt rc out
 orders=$((run_rate * run_duration))
 side_orders=$((orders + 32))
 refresh_ms=$(((run_duration + 20) * 1000))
 IFS=, read -r symbol_id _ <<<"$SYMBOLS"
 # Every stress order is sized to the symbol's minimum notional at its own
 # price, so one order takes `lots_per_order` lots. The price is known only
 # once the maker has quoted: the first pass provisions one lot per order,
 # the sizing step reports the lots an order needs (exit 3), and the second
 # pass provisions that many.
 for attempt in 1 2 3
 do
  curl -fsS -X POST 'http://127.0.0.1:49171/api/maker/config' \
   -H 'x-confirm: yes' \
   --data-urlencode "qty=$((side_orders * lots_per_order))" \
   --data-urlencode "symbol_id=$symbol_id" \
   --data-urlencode "refresh_ms=$refresh_ms" \
   --data-urlencode 'levels=1' >/dev/null || return 2
  curl -fsS -X POST 'http://127.0.0.1:49171/api/maker/start?confirm=yes' \
   -H 'x-confirm: yes' >/dev/null || return 2
  rc=0
  out=$(size_orders "$side_orders" "$lots_per_order") || rc=$?
  case $rc in
   0) SYMBOL_CONFIGS=$out; return 0 ;;
   3) lots_per_order=$out ;;
   *) return 2 ;;
  esac
 done
 echo '[latency-publish] the book could not be provisioned for the minimum order size' >&2
 return 2
}
size_orders() {
 python3 - "$SYMBOLS" "$1" "$2" "$PWD/rsx-playground" <<'PY'
import json
import sys
import time
import urllib.error
import urllib.request

symbols = [int(value) for value in sys.argv[1].split(",")]
side_orders = int(sys.argv[2])
lots_per_order = int(sys.argv[3])
sys.path.insert(0, sys.argv[4])
import notional

with urllib.request.urlopen("http://127.0.0.1:49171/v1/symbols") as response:
    catalog = {
        row["id"]: row for row in json.load(response)["symbols"]
    }
missing = [symbol for symbol in symbols if symbol not in catalog]
if missing:
    raise SystemExit(f"unknown scenario symbols: {missing}")

for _ in range(40):
    resolved = {}
    needed_lots = 0
    for symbol in symbols:
        try:
            with urllib.request.urlopen(
                f"http://127.0.0.1:49171/api/bbo/{symbol}", timeout=2
            ) as response:
                bbo = json.load(response)
        except urllib.error.HTTPError as exc:
            # /api/bbo/{id} returns 404 ("no bbo for symbol") until the
            # maker has quoted at least once — expected while it warms
            # up, not a fatal error. Treat as "not ready yet" and retry.
            if exc.code == 404:
                break
            raise
        meta = catalog[symbol]
        tick = int(meta["tick_size"])
        lot = int(meta["lot_size"])
        bid = int(bbo.get("bid_px", 0))
        ask = int(bbo.get("ask_px", 0))
        if (
            bbo.get("source") != "live"
            or bid <= 0 or ask <= bid
            or bid % tick or ask % tick
        ):
            break
        floor = notional.catalog_min_notional(meta)
        # The stress client sizes each order to the symbol minimum at its
        # own price; the book must hold side_orders of them per side.
        order_lots = notional.crossing_order_lots(bid, ask, lot, floor)
        if order_lots > lots_per_order:
            needed_lots = max(needed_lots, order_lots)
            resolved[str(symbol)] = None
            continue
        enough = side_orders * lots_per_order * lot
        if (
            int(bbo.get("bid_qty", 0)) < enough
            or int(bbo.get("ask_qty", 0)) < enough
        ):
            break
        resolved[str(symbol)] = {
            "tick_size": tick,
            "lot_size": lot,
            "min_notional": floor,
            "bid_price": bid,
            "ask_price": ask,
        }
    if len(resolved) == len(symbols):
        if needed_lots:
            # 25% headroom for a re-quote at a slightly lower price.
            print(-(-needed_lots * 5 // 4))
            raise SystemExit(3)
        print(json.dumps(resolved, separators=(",", ":")))
        break
    time.sleep(0.25)
else:
    raise SystemExit("sufficient live two-sided liquidity unavailable for every symbol")
PY
}
publish() {
python3 - "$BASELINE" "$1" <<'PY'
import json,os,sys,tempfile,time
path,row=sys.argv[1],json.loads(sys.argv[2]); r=row['report']; m=r['metrics']; latency=r['latency_us']
try: data=json.load(open(path))
except (OSError,json.JSONDecodeError): data={}
data['e2e_us']={'p50':latency['p50'],'p95':latency['p95'],'p99':latency['p99'],'p99_9':latency['p99_9'],'max':latency['max'],
 'n':latency['samples'],'accepted':m['accepted'],'accepted_throughput':m['accepted']/m['elapsed_sec'],
 'target_rate':r['config']['target_rate'],'achieved_rate':m['achieved_rate'],'offered':m['offered'],'submitted':m['submitted'],
 'rejected':m['rejected'],'timed_out':m['timed_out'],'pending':m['pending'],'errors':m['errors'],
 'send_errors':m['send_errors'],'terminal_ratio':row['terminal_ratio'],'throughput_ratio':row['throughput_ratio'],
 'environment':'shared-host','valid':True,
 'timestamp_legs':'normal F/U replies; gateway, risk, matching, completion, and reply-construction clocks',
 'ts':int(time.time())}
data['e2e_us']['server_latency_ns']=r.get('server_latency_ns',{})
data['e2e_us']['scenario']=os.environ.get('RSX_STRESS_SCENARIO','unknown')
data['e2e_us']['symbols']=r.get('config',{}).get('symbols',[])
data['e2e_us']['per_symbol']=r.get('per_symbol',{})
fd,tmp=tempfile.mkstemp(prefix='.bench-baseline.',dir=os.path.dirname(path) or '.',text=True)
with os.fdopen(fd,'w') as f: json.dump(data,f,indent=2,sort_keys=True); f.write('\n')
os.replace(tmp,path); print(f'[latency-publish] published valid result to {path}',file=sys.stderr)
PY
}

summarize() {
python3 - "$BASELINE" "$1" <<'PY'
import json,os,sys,tempfile
path,summary=sys.argv[1],json.loads(sys.argv[2]); data=json.load(open(path))
if data.get('e2e_us',{}).get('valid') is not True: raise SystemExit('refusing to summarize invalid baseline')
data['e2e_us']['run_summary']=summary
fd,tmp=tempfile.mkstemp(prefix='.bench-baseline.',dir=os.path.dirname(path) or '.',text=True)
with os.fdopen(fd,'w') as f: json.dump(data,f,indent=2,sort_keys=True); f.write('\n')
os.replace(tmp,path)
PY
}

run_one() {
 local run_rate=$1 run_duration=$2 need=$3 before report rc row
 seed_crossing_book "$run_rate" "$run_duration"
 before=$(find "$REPORT_DIR" -name 'stress-*.json' -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2- || true)
 set +e
  RSX_STRESS_GW_URL="$GW_URL" RSX_STRESS_RATE="$run_rate" RSX_STRESS_DURATION="$run_duration" \
  RSX_STRESS_SCENARIO="$SCENARIO" \
  RSX_STRESS_USERS="$USERS" RSX_STRESS_TARGET_P99=9223372036854775807 \
  RSX_STRESS_CONNECTIONS="$CONNECTIONS" \
  RSX_STRESS_PROCESSES="$PROCESSES" \
  RSX_STRESS_SYMBOLS="$SYMBOLS" RSX_STRESS_SYMBOL_CONFIGS="$SYMBOL_CONFIGS" \
  RSX_STRESS_LOT_SIZE="$LOT_SIZE" \
  RSX_GW_JWT_SECRET="$JWT_SECRET" \
  RSX_STRESS_REPORT_DIR="$REPORT_DIR" python3 rsx-playground/stress.py >&2
 rc=$?
 set -e
 report=$(find "$REPORT_DIR" -name 'stress-*.json' -printf '%T@ %p\n' | sort -n | tail -1 | cut -d' ' -f2-)
 [ -n "$report" ] && [ "$report" != "$before" ] || { echo '[latency-publish] no new report' >&2; return 1; }
 [ "$rc" -eq 0 ] || echo "[latency-publish] stress exit=$rc; checking validity" >&2
 row=$(validate "$report" "$need" "$MIN_RATIO") || return 1
 publish "$row"
 printf '%s\n' "$row"
}

case "$MODE" in
 single)
  run_one "$RATE" "$DURATION" "$MIN_SAMPLES" >/dev/null
  summarize "{\"mode\":\"single\",\"rate\":$RATE}"
  ;;
 staircase)
  stable=
 for step_rate in $RATES; do
   step_duration=$(( (N + step_rate - 1) / step_rate ))
   echo "[latency-publish] staircase rate=$step_rate duration=${step_duration}s"
   if run_one "$step_rate" "$step_duration" "$MIN_SAMPLES" >/dev/null; then stable=$step_rate; else break; fi
  done
  [ -n "$stable" ] || { echo '[latency-publish] no valid staircase step' >&2; exit 1; }
  summarize "{\"mode\":\"staircase\",\"highest_stable_rate\":$stable}"
  echo "[latency-publish] highest stable rate=$stable"
 ;;
 connections)
  points=()
  valid_points=0
  for connections in $CONNECTION_STEPS
  do
   CONNECTIONS=$connections
   if [[ -z $USERS_CONFIGURED ]]
   then
    USERS=$connections
   fi
   stable=
   for step_rate in $RATES
   do
    step_duration=$(( (N + step_rate - 1) / step_rate ))
    echo "[latency-publish] connections=$connections rate=$step_rate duration=${step_duration}s"
    if run_one "$step_rate" "$step_duration" "$MIN_SAMPLES" >/dev/null
    then
     stable=$step_rate
    else
     break
    fi
   done
   if [ -n "$stable" ]
   then
    points+=("$connections:$stable")
    valid_points=$((valid_points + 1))
   else
    echo "[latency-publish] no valid rate at connections=$connections" >&2
    points+=("$connections:null")
   fi
  done
  (( valid_points > 0 )) || { echo '[latency-publish] no valid connection points' >&2; exit 1; }
  summary=$(python3 - "${points[@]}" <<'PY'
import json
import sys
points = []
for value in sys.argv[1:]:
    connections, rate = value.split(":", 1)
    points.append({
        "connections": int(connections),
        "highest_stable_rate": None if rate == "null" else int(rate),
    })
print(json.dumps({"mode": "connections", "points": points}))
PY
)
  summarize "$summary"
  echo "[latency-publish] connection sweep: ${points[*]}"
 ;;
 long)
  if [ -z "$RATE_CONFIGURED" ] && [ -f "$BASELINE" ]; then
   RATE=$(python3 - "$BASELINE" "$RATE" <<'PY'
import json,sys
try: print(int(json.load(open(sys.argv[1]))['e2e_us']['target_rate']))
except (OSError,ValueError,TypeError,KeyError): print(sys.argv[2])
PY
)
  fi
  p99s=()
  long_need=${LONG_MIN_SAMPLES:-$MIN_SAMPLES}
  for ((i=1; i<=REPEATS; i++)); do
   echo "[latency-publish] long run=$i/$REPEATS rate=$RATE duration=${LONG_DURATION}s"
   row=$(run_one "$RATE" "$LONG_DURATION" "$long_need")
   p99s+=("$(python3 - "$row" <<'PY'
import json,sys
print(json.loads(sys.argv[1])['report']['latency_us']['p99'])
PY
)")
  done
  spread=$(python3 - "$MAX_P99_SPREAD" "${p99s[@]}" <<'PY'
import sys
limit=float(sys.argv[1]); values=list(map(float,sys.argv[2:]))
spread=(max(values)-min(values))/min(values) if min(values) else (0 if max(values)==0 else float('inf'))
if spread > limit: raise SystemExit(f'long-run p99 instability {spread:.2%} exceeds {limit:.2%}')
print(spread)
PY
)
  echo "[latency-publish] long p99 spread=$spread limit=$MAX_P99_SPREAD"
  p99_json=$(printf '%s\n' "${p99s[@]}" | python3 -c 'import json,sys; print(json.dumps([float(x) for x in sys.stdin]))')
  summarize "{\"mode\":\"long\",\"rate\":$RATE,\"duration\":$LONG_DURATION,\"repeats\":$REPEATS,\"p99_values\":$p99_json,\"p99_spread\":$spread}"
  ;;
esac
