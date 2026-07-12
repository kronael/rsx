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
USERS=${USERS:-10}
GW_URL=${RSX_STRESS_GW_URL:-ws://127.0.0.1:8088}
VALIDATE=""
[ "${1:-}" != "--validate-report" ] || { [ "$#" -eq 2 ] || exit 2; VALIDATE=$2; }
[ "${1:-}" != "--validate-mode" ] || { [ "$#" -eq 1 ] || exit 2; MODE_VALIDATE=1; }
[ "$#" -eq 0 ] || [ -n "$VALIDATE" ] || [ "${MODE_VALIDATE:-}" = 1 ] || { echo "unknown argument: $1" >&2; exit 2; }
mkdir -p "$REPORT_DIR"

validate_mode() {
 case "$MODE" in single|staircase|long) ;; *) echo "invalid MODE=$MODE (expected single, staircase, or long)" >&2; return 2;; esac
 [[ "$RATE" =~ ^[1-9][0-9]*$ ]] || { echo 'RATE must be a positive integer' >&2; return 2; }
 [[ "$LONG_DURATION" =~ ^[1-9][0-9]*$ ]] || { echo 'LONG_DURATION must be a positive integer' >&2; return 2; }
 [[ "$REPEATS" =~ ^[1-9][0-9]*$ ]] || { echo 'REPEATS must be a positive integer' >&2; return 2; }
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
for k in ('p50','p95','p99','p99_9','max'):
 v=lat.get(k)
 if not isinstance(v,(int,float)) or isinstance(v,bool) or not math.isfinite(v) or v<0: bad.append(f'{k} unavailable')
nonperf=[x for x in d.get('failures',[]) if not str(x).startswith('p99=')]
if nonperf: bad.append('stress correctness failed: '+'; '.join(nonperf))
if bad: print('invalid measurement: '+'; '.join(dict.fromkeys(bad)),file=sys.stderr); raise SystemExit(1)
print(json.dumps({'valid':True,'terminal_ratio':terminal,'throughput_ratio':throughput,'report':d}))
PY
}

validate_mode
if [ "${MODE_VALIDATE:-}" = 1 ]; then exit; fi
if [ -n "$VALIDATE" ]; then validate "$VALIDATE" "$MIN_SAMPLES" "$MIN_RATIO" >/dev/null; exit; fi

curl -fsS -m 2 http://127.0.0.1:49171/healthz >/dev/null || { echo '[latency-publish] playground unhealthy' >&2; exit 2; }
curl -fsS -X POST 'http://127.0.0.1:49171/api/maker/start?confirm=yes' \
 -H 'x-confirm: yes' >/dev/null || { echo '[latency-publish] maker unavailable' >&2; exit 2; }
publish() {
python3 - "$BASELINE" "$1" <<'PY'
import json,os,sys,tempfile,time
path,row=sys.argv[1],json.loads(sys.argv[2]); r=row['report']; m=r['metrics']; l=r['latency_us']
try: data=json.load(open(path))
except (OSError,json.JSONDecodeError): data={}
data['e2e_us']={'p50':l['p50'],'p95':l['p95'],'p99':l['p99'],'p99_9':l['p99_9'],'max':l['max'],
 'n':l['samples'],'accepted':m['accepted'],'accepted_throughput':m['accepted']/m['elapsed_sec'],
 'target_rate':r['config']['target_rate'],'achieved_rate':m['achieved_rate'],'offered':m['offered'],'submitted':m['submitted'],
 'rejected':m['rejected'],'timed_out':m['timed_out'],'pending':m['pending'],'errors':m['errors'],
 'send_errors':m['send_errors'],'terminal_ratio':row['terminal_ratio'],'throughput_ratio':row['throughput_ratio'],
 'environment':'shared-host','valid':True,'timestamp_legs':'not emitted (spec 59 planned)','ts':int(time.time())}
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
 before=$(find "$REPORT_DIR" -name 'stress-*.json' -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2- || true)
 set +e
  RSX_STRESS_GW_URL="$GW_URL" RSX_STRESS_RATE="$run_rate" RSX_STRESS_DURATION="$run_duration" \
  RSX_STRESS_USERS="$USERS" RSX_STRESS_TARGET_P99=9223372036854775807 \
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
   step_duration=$DURATION
   echo "[latency-publish] staircase rate=$step_rate duration=${step_duration}s"
   if run_one "$step_rate" "$step_duration" "$MIN_SAMPLES" >/dev/null; then stable=$step_rate; else break; fi
  done
  [ -n "$stable" ] || { echo '[latency-publish] no valid staircase step' >&2; exit 1; }
  summarize "{\"mode\":\"staircase\",\"highest_stable_rate\":$stable}"
  echo "[latency-publish] highest stable rate=$stable"
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
