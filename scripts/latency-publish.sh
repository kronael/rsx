#!/usr/bin/env bash
# Publish sustained external WebSocket latency using the existing stress client.
set -euo pipefail
cd "$(dirname "$0")/.."

BASELINE=${BASELINE:-bench-baseline.json}
REPORT_DIR=${REPORT_DIR:-rsx-playground/tmp/bench}
RATE=${RATE:-1000}
N=${N:-2000}
DURATION=${DURATION:-$(( (N + RATE - 1) / RATE ))}
MIN_SAMPLES=${MIN_SAMPLES:-$N}
MIN_RATIO=${MIN_RATIO:-0.95}
USERS=${USERS:-10}
GW_URL=${RSX_STRESS_GW_URL:-ws://127.0.0.1:8088}
VALIDATE=""
[ "${1:-}" != "--validate-report" ] || { [ "$#" -eq 2 ] || exit 2; VALIDATE=$2; }
[ "$#" -eq 0 ] || [ -n "$VALIDATE" ] || { echo "unknown argument: $1" >&2; exit 2; }
mkdir -p "$REPORT_DIR"

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

if [ -n "$VALIDATE" ]; then validate "$VALIDATE" "$MIN_SAMPLES" "$MIN_RATIO" >/dev/null; exit; fi

curl -fsS -m 2 http://127.0.0.1:49171/healthz >/dev/null || { echo '[latency-publish] playground unhealthy' >&2; exit 2; }
curl -fsS -X POST 'http://127.0.0.1:49171/api/maker/start?confirm=yes' \
 -H 'x-confirm: yes' >/dev/null || { echo '[latency-publish] maker unavailable' >&2; exit 2; }
before=$(find "$REPORT_DIR" -name 'stress-*.json' -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2- || true)
set +e
RSX_STRESS_GW_URL="$GW_URL" RSX_STRESS_RATE="$RATE" RSX_STRESS_DURATION="$DURATION" \
 RSX_STRESS_USERS="$USERS" RSX_STRESS_TARGET_P99=9223372036854775807 \
 RSX_STRESS_REPORT_DIR="$REPORT_DIR" python3 rsx-playground/stress.py
rc=$?
set -e
report=$(find "$REPORT_DIR" -name 'stress-*.json' -printf '%T@ %p\n' | sort -n | tail -1 | cut -d' ' -f2-)
[ -n "$report" ] && [ "$report" != "$before" ] || { echo '[latency-publish] no new report' >&2; exit 1; }
[ "$rc" -eq 0 ] || echo "[latency-publish] stress exit=$rc; checking validity" >&2
row=$(validate "$report" "$MIN_SAMPLES" "$MIN_RATIO")

# Invalid runs exit above, before the atomic baseline update.
python3 - "$BASELINE" "$row" <<'PY'
import json,os,sys,tempfile,time
path,row=sys.argv[1],json.loads(sys.argv[2]); r=row['report']; m=r['metrics']; l=r['latency_us']
try: data=json.load(open(path))
except (OSError,json.JSONDecodeError): data={}
data['e2e_us']={'p50':l['p50'],'p95':l['p95'],'p99':l['p99'],'p99_9':l['p99_9'],'max':l['max'],
 'n':l['samples'],'accepted':m['accepted'],'accepted_throughput':m['accepted']/m['elapsed_sec'],
 'achieved_rate':m['achieved_rate'],'offered':m['offered'],'submitted':m['submitted'],
 'rejected':m['rejected'],'timed_out':m['timed_out'],'pending':m['pending'],'errors':m['errors'],
 'send_errors':m['send_errors'],'terminal_ratio':row['terminal_ratio'],'throughput_ratio':row['throughput_ratio'],
 'environment':'shared-host','valid':True,'timestamp_legs':'not emitted (spec 59 planned)','ts':int(time.time())}
fd,tmp=tempfile.mkstemp(prefix='.bench-baseline.',dir=os.path.dirname(path) or '.',text=True)
with os.fdopen(fd,'w') as f: json.dump(data,f,indent=2,sort_keys=True); f.write('\n')
os.replace(tmp,path); print(f'[latency-publish] published valid result to {path}')
PY
