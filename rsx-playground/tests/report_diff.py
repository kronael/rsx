#!/usr/bin/env python3
"""Diff gate-3 JSON reports: current vs previous run.

Reads:
  tmp/gate-3-report.json       current run (written by conftest.py)
  tmp/gate-3-report.prev.json  previous run (rotated by conftest.py)

Emits machine-readable JSON to stdout:
  {
    "current":  { "total": N, "passed": N, "failed": N, "by_class": {...} },
    "previous": { ... } | null,
    "regression": [{"test": ..., "class": ...}],   # newly failing
    "recovery":   [{"test": ..., "class": ...}],   # newly passing
    "new_in_class": {"processes": +2, "risk": -1, ...}
  }

Exit codes:
  0  no regressions
  1  regressions found (newly failing tests)
  2  report file missing
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).parent
REPORT_DIR = HERE.parent / "tmp"
CURRENT = REPORT_DIR / "gate-3-report.json"
PREV = REPORT_DIR / "gate-3-report.prev.json"


def load(path: Path) -> dict | None:
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except Exception:
        return None


def failed_tests(report: dict) -> dict[str, str]:
    """Return {test_id: endpoint_class} for all failed tests."""
    out = {}
    for ec, data in report.get("by_class", {}).items():
        for f in data.get("failures", []):
            out[f["test"]] = ec
    return out


def main():
    cur = load(CURRENT)
    if cur is None:
        print(
            json.dumps({"error": f"report not found: {CURRENT}"}),
            file=sys.stderr,
        )
        sys.exit(2)

    prev = load(PREV)

    cur_fails = failed_tests(cur)
    prev_fails = failed_tests(prev) if prev else {}

    # Regression: failing now but not before
    regression = [
        {"test": t, "class": c}
        for t, c in cur_fails.items()
        if t not in prev_fails
    ]

    # Recovery: was failing before, now passing
    recovery = [
        {"test": t, "class": c}
        for t, c in prev_fails.items()
        if t not in cur_fails
    ]

    # Delta by endpoint class
    all_classes = set(cur.get("by_class", {})) | set(
        (prev or {}).get("by_class", {})
    )
    delta: dict[str, int] = {}
    for ec in sorted(all_classes):
        cur_f = cur.get("by_class", {}).get(ec, {}).get("failed", 0)
        prev_f = (prev or {}).get("by_class", {}).get(ec, {}).get(
            "failed", 0
        )
        if cur_f != prev_f:
            delta[ec] = cur_f - prev_f

    result = {
        "current": {
            "run_ts": cur.get("run_ts"),
            "total": cur.get("total", 0),
            "passed": cur.get("passed", 0),
            "failed": cur.get("failed", 0),
            "by_class": {
                ec: {
                    "passed": v.get("passed", 0),
                    "failed": v.get("failed", 0),
                }
                for ec, v in cur.get("by_class", {}).items()
            },
        },
        "previous": (
            {
                "run_ts": prev.get("run_ts"),
                "total": prev.get("total", 0),
                "passed": prev.get("passed", 0),
                "failed": prev.get("failed", 0),
            }
            if prev
            else None
        ),
        "regression": regression,
        "recovery": recovery,
        "delta_by_class": delta,
    }

    print(json.dumps(result, indent=2))

    if regression:
        print(
            f"\n[report_diff] REGRESSIONS: {len(regression)} newly failing test(s)",
            file=sys.stderr,
        )
        for r in regression:
            print(f"  [{r['class']}] {r['test']}", file=sys.stderr)
        sys.exit(1)

    print(
        f"[report_diff] OK: {cur['passed']}/{cur['total']} passed, "
        f"{len(recovery)} recovered",
        file=sys.stderr,
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
