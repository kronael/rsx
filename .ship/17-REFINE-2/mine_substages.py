#!/usr/bin/env python3
"""Mine per-stage and per-sub-stage latencies from ME, risk,
and gateway logs after a latency-publish run.

Joins by oid, filters coherent traces (every emission shares
the same t0_ns), prints p50/p95/p99 for every stage seen.
"""
from __future__ import annotations
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOG_DIR = ROOT / "log"

ANSI = re.compile(r"\x1b\[[0-9;]*m")

LINE_RE = re.compile(
    r'stage="([^"]+)"\s+'
    r'(?:conn_id=\d+\s+(?:drained_n=\d+\s+)?)?'
    r'(?:oid="?([0-9a-f]+)"?\s+)?'
    r't_us=(\d+)\s+t0_ns=(\d+)'
)

# Canonical sub-stage order (top -> bottom)
STAGES = [
    "gateway_in",
    "risk_in",
    "me_in",
    "me_dedup_done",
    "me_wal_accepted_done",
    "me_match_done",
    "me_wal_events_done",
    "me_index_done",
    "me_out",
    "risk_out",
    "risk_cmp_send_done",
    "gateway_cmp_recv",
    "gateway_route_serialize_done",
    "gateway_out",
    "gateway_route_push_done",
]


def parse_log(path: Path):
    records = []
    if not path.exists():
        return records
    with path.open() as f:
        for raw in f:
            line = ANSI.sub("", raw)
            m = LINE_RE.search(line)
            if not m:
                continue
            stage, oid, t_us, t0_ns = m.groups()
            if oid is None:
                continue  # loop-level (drain start/done): no oid join
            records.append((oid, stage, int(t_us), int(t0_ns)))
    return records


def percentiles(values, ps):
    if not values:
        return {p: None for p in ps}
    values = sorted(values)
    out = {}
    for p in ps:
        if len(values) == 1:
            out[p] = values[0]
            continue
        k = (len(values) - 1) * (p / 100)
        f = int(k)
        c = min(f + 1, len(values) - 1)
        out[p] = values[f] + (values[c] - values[f]) * (k - f)
    return out


def main():
    files = [
        LOG_DIR / "gw-0.log",
        LOG_DIR / "risk-0.log",
        LOG_DIR / "me-pengu.log",
    ]
    all_recs = []
    for f in files:
        recs = parse_log(f)
        print(f"  {f.name}: {len(recs)} traced lines", file=sys.stderr)
        all_recs.extend(recs)

    by_oid = defaultdict(dict)
    by_oid_t0 = defaultdict(dict)
    for oid, stage, t_us, t0_ns in all_recs:
        by_oid[oid][stage] = t_us
        by_oid_t0[oid][stage] = t0_ns

    # Coherent: must have all of gateway_in/risk_in/me_in/me_out/risk_out/gateway_out
    # and all share the same t0_ns.
    required = {"gateway_in", "risk_in", "me_in", "me_out",
                "risk_out", "gateway_out"}
    coherent = []
    for oid, stages in by_oid.items():
        if not required.issubset(stages.keys()):
            continue
        t0s = {by_oid_t0[oid][s] for s in required}
        if len(t0s) != 1:
            continue
        coherent.append(oid)

    print(f"\ncoherent traces: {len(coherent)}", file=sys.stderr)

    # Aggregate t_us per stage from coherent traces only
    by_stage = defaultdict(list)
    for oid in coherent:
        for stage, t_us in by_oid[oid].items():
            by_stage[stage].append(t_us)

    print(f"\n{'stage':<35}{'count':>8}{'p50':>10}{'p95':>10}{'p99':>10}")
    print("-" * 73)
    for stage in STAGES:
        if stage not in by_stage:
            print(f"{stage:<35}{'-':>8}{'-':>10}{'-':>10}{'-':>10}")
            continue
        vals = by_stage[stage]
        ps = percentiles(vals, [50, 95, 99])
        print(f"{stage:<35}{len(vals):>8}"
              f"{ps[50]:>10.0f}{ps[95]:>10.0f}{ps[99]:>10.0f}")


if __name__ == "__main__":
    main()
