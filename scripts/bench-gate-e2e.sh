#!/usr/bin/env bash
# Validate E2E accounting and samples, then regress the tail (p99).
set -euo pipefail
cd "$(dirname "$0")/.."
THRESHOLD=${THRESHOLD:-10}; N=${N:-200}; MIN_SAMPLES=${MIN_SAMPLES:-$N}
REFERENCE=${REFERENCE:-bench-reference.json}; BASELINE=${BASELINE:-bench-e2e-latest.json}; SAVE=0
for arg in "$@"; do case "$arg" in --save-reference) SAVE=1;; *) echo "unknown arg: $arg" >&2; exit 2;; esac; done
if [ "${SKIP_RUN:-0}" != 1 ]; then
 N="$N" MIN_SAMPLES="$MIN_SAMPLES" BASELINE="$BASELINE" bash scripts/latency-publish.sh
fi
python3 - "$BASELINE" "$REFERENCE" "$THRESHOLD" "$SAVE" "$MIN_SAMPLES" <<'PY'
import json,math,os,sys,tempfile,time
b,r,t,save,need=sys.argv[1],sys.argv[2],float(sys.argv[3]),sys.argv[4]=='1',int(sys.argv[5])
try: e=json.load(open(b))['e2e_us']
except (OSError,json.JSONDecodeError,KeyError,TypeError) as x:
 print(f'[bench-gate-e2e] measurement_invalid=true: {x}',file=sys.stderr); raise SystemExit(2)
keys=('p50','p95','p99','p99_9','max','n','accepted','offered','submitted')
missing=[k for k in keys if e.get(k) is None]
bad=missing or e.get('valid') is not True or e.get('n',0)<need or e.get('accepted')!=e.get('n')
bad=bad or bool(e.get('pending')) or e.get('offered') != e.get('submitted',0)+e.get('send_errors',0)
if bad:
 print(f"[bench-gate-e2e] measurement_invalid=true missing={missing} n={e.get('n')} accepted={e.get('accepted')}",file=sys.stderr); raise SystemExit(2)
if save:
 out={'e2e_us':{k:e[k] for k in ('p50','p95','p99','p99_9','max','n')},'ts':int(time.time()),
      '_comment':f'Sealed valid E2E reference; p99 threshold {t:g}%.'}
 fd,tmp=tempfile.mkstemp(prefix='.bench-reference.',dir=os.path.dirname(r) or '.',text=True)
 with os.fdopen(fd,'w') as f: json.dump(out,f,indent=2,sort_keys=True); f.write('\n')
 os.replace(tmp,r); print(f'[bench-gate-e2e] saved valid reference to {r}'); raise SystemExit
try: rp=json.load(open(r))['e2e_us']['p99']
except (OSError,json.JSONDecodeError,KeyError,TypeError) as x:
 print(f'[bench-gate-e2e] invalid reference: {x}',file=sys.stderr); raise SystemExit(2)
if not isinstance(rp,(int,float)) or not math.isfinite(rp) or rp<=0:
 print('[bench-gate-e2e] invalid reference p99',file=sys.stderr); raise SystemExit(2)
pct=(e['p99']/rp-1)*100
print(f"[bench-gate-e2e] p99 current={e['p99']}us reference={rp}us change={pct:.2f}%")
if pct>t:
 print('[bench-gate-e2e] performance_target_miss=true measurement_invalid=false',file=sys.stderr); raise SystemExit(1)
print('[bench-gate-e2e] performance_target_miss=false measurement_invalid=false')
PY
