#!/usr/bin/env python3
"""Compute per-leg deltas (p50/p95/p99) between adjacent
sub-stages on coherent traces. Adjacent means: for each oid,
delta[B] = t_us[B] - t_us[A] where (A, B) is a defined pair.
"""
from __future__ import annotations
import re
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

PAIRS = [
    ("gateway_in", "risk_in"),
    ("risk_in", "me_in"),
    ("me_in", "me_dedup_done"),
    ("me_dedup_done", "me_wal_accepted_done"),
    ("me_wal_accepted_done", "me_match_done"),
    ("me_match_done", "me_wal_events_done"),
    ("me_wal_events_done", "me_index_done"),
    ("me_index_done", "me_out"),
    ("me_in", "me_out"),
    ("me_out", "risk_out"),
    ("risk_out", "risk_cmp_send_done"),
    ("risk_cmp_send_done", "gateway_cmp_recv"),
    ("gateway_cmp_recv", "gateway_route_serialize_done"),
    ("gateway_route_serialize_done", "gateway_out"),
    ("gateway_out", "gateway_route_push_done"),
    ("risk_out", "gateway_out"),
    ("gateway_in", "gateway_out"),
]


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
    by_oid = defaultdict(dict)
    by_oid_t0 = defaultdict(dict)
    for f in files:
        if not f.exists():
            continue
        with f.open() as fh:
            for raw in fh:
                line = ANSI.sub("", raw)
                m = LINE_RE.search(line)
                if not m:
                    continue
                stage, oid, t_us, t0_ns = m.groups()
                if oid is None:
                    continue
                by_oid[oid][stage] = int(t_us)
                by_oid_t0[oid][stage] = int(t0_ns)

    # coherent on the 6 macro stages
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
    print(f"coherent traces: {len(coherent)}", file=sys.stderr)

    print(f"\n{'leg':<55}{'n':>6}{'p50':>10}{'p95':>10}{'p99':>10}")
    print("-" * 91)
    for a, b in PAIRS:
        deltas = []
        for oid in coherent:
            sa = by_oid[oid].get(a)
            sb = by_oid[oid].get(b)
            if sa is None or sb is None:
                continue
            deltas.append(sb - sa)
        if not deltas:
            print(f"{a}->{b:<{55-len(a)-2}}{'-':>6}{'-':>10}{'-':>10}{'-':>10}")
            continue
        ps = percentiles(deltas, [50, 95, 99])
        leg = f"{a} -> {b}"
        print(f"{leg:<55}{len(deltas):>6}"
              f"{ps[50]:>10.0f}{ps[95]:>10.0f}{ps[99]:>10.0f}")


if __name__ == "__main__":
    main()
