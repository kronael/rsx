#!/usr/bin/env bash
# Validate E2E accounting and regress stable client/server percentiles.
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
legs=('gateway_to_risk','risk_to_me','matching','match_done_to_gateway','internal')
server=e.get('server_latency_ns',{})
for leg in legs:
 row=server.get(leg,{})
 if row.get('samples',0)<need:
  missing.append(f'{leg}.samples')
 for percentile in ('p50','p99'):
  value=row.get(percentile)
  if not isinstance(value,(int,float)) or isinstance(value,bool) or not math.isfinite(value) or value<0:
   missing.append(f'{leg}.{percentile}')
bad=bad or bool(missing)
if bad:
 print(f"[bench-gate-e2e] measurement_invalid=true missing={missing} n={e.get('n')} accepted={e.get('accepted')}",file=sys.stderr); raise SystemExit(2)
if save:
 out={'e2e_us':{k:e[k] for k in ('p50','p99','n')},'ts':int(time.time()),
      '_comment':f'Sealed valid like-for-like reference; p50/p99 threshold {t:g}%.'}
 out['e2e_us']['server_latency_ns']={
  leg:{k:server[leg][k] for k in ('samples','p50','p99')} for leg in legs
 }
 fd,tmp=tempfile.mkstemp(prefix='.bench-reference.',dir=os.path.dirname(r) or '.',text=True)
 with os.fdopen(fd,'w') as f: json.dump(out,f,indent=2,sort_keys=True); f.write('\n')
 os.replace(tmp,r); print(f'[bench-gate-e2e] saved valid reference to {r}'); raise SystemExit
try: reference=json.load(open(r))['e2e_us']
except (OSError,json.JSONDecodeError,KeyError,TypeError) as x:
 print(f'[bench-gate-e2e] invalid reference: {x}',file=sys.stderr); raise SystemExit(2)
comparisons=[('client_us', percentile, e[percentile], reference.get(percentile))
             for percentile in ('p50','p99')]
reference_server=reference.get('server_latency_ns',{})
for leg in legs:
 for percentile in ('p50','p99'):
  comparisons.append((f'{leg}_ns', percentile, server[leg][percentile],
                      reference_server.get(leg,{}).get(percentile)))
miss=False
for name,percentile,current,ref in comparisons:
 if not isinstance(ref,(int,float)) or isinstance(ref,bool) or not math.isfinite(ref) or ref<0:
  print(f'[bench-gate-e2e] invalid reference {name}.{percentile}',file=sys.stderr); raise SystemExit(2)
 change=(current/ref-1)*100 if ref else (0 if current==0 else float('inf'))
 print(f'[bench-gate-e2e] {name}.{percentile} current={current} reference={ref} change={change:.2f}%')
 miss=miss or change>t
if miss:
 print('[bench-gate-e2e] performance_target_miss=true measurement_invalid=false',file=sys.stderr); raise SystemExit(1)
print('[bench-gate-e2e] performance_target_miss=false measurement_invalid=false')
PY
