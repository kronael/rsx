#!/usr/bin/env python3
"""Meta-orchestration guard: block new meta tasks until
product-critical failing Playwright IDs decrease across two
consecutive fresh acceptance-bundle cycles.

A "fresh cycle" is a new acceptance-bundle.json with a different
commit SHA than the previously recorded cycle.  When the bundle is
regenerated (new SHA), it is appended to the trend file.  We keep
the last 3 cycles; two consecutive decreases are required to unblock.

State file: rsx-playground/tmp/failing-trend.json
  {
    "cycles": [
      {"sha": "abc1234", "ts": 1700000000, "count": 45},
      {"sha": "def5678", "ts": 1700001000, "count": 40},
      {"sha": "ghi9012", "ts": 1700002000, "count": 35}
    ]
  }

Meta-orchestration task keywords (matched against task description):
  acceptance, bundle, ci-full, publish, progress, release-gate,
  exit-criteria, task-report, gen-release-truth, regen-progress

Exit codes:
  0   allowed (no data, or 2 consecutive decreases observed)
  1   blocked (stagnant or rising failing count over 2 cycles)
  2   acceptance bundle missing or unreadable
  3   dry-run: would block (printed, nothing written)

Usage:
  python3 scripts/meta-guard.py              # update trend + check
  python3 scripts/meta-guard.py --dry-run    # report only, no writes
  python3 scripts/meta-guard.py --verbose    # show trend detail
  python3 scripts/meta-guard.py --status     # print trend, exit 0
"""

import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).parent.parent
BUNDLE_PATH = ROOT / "rsx-playground" / "tmp" / "acceptance-bundle.json"
TREND_PATH = ROOT / "rsx-playground" / "tmp" / "failing-trend.json"

# Keep at most this many cycles in the trend file.
MAX_CYCLES = 3

# Keywords that identify a meta-orchestration task description.
META_KEYWORDS = {
    "acceptance",
    "bundle",
    "publish",
    "progress",
    "release",
    "exit-criteria",
    "task-report",
    "regen",
    "local-validate",
    "ci-full",
    "gen-release",
}


def is_meta_task(description: str) -> bool:
    """Return True if the task is a meta-orchestration task."""
    low = description.lower()
    return any(kw in low for kw in META_KEYWORDS)


def load_bundle() -> dict:
    if not BUNDLE_PATH.exists():
        print(
            "[meta-guard] ERROR: acceptance bundle missing: "
            f"{BUNDLE_PATH}\n"
            "  Run: python3 scripts/acceptance-bundle.py",
            file=sys.stderr,
        )
        sys.exit(2)
    try:
        return json.loads(BUNDLE_PATH.read_text())
    except Exception as exc:
        print(
            f"[meta-guard] ERROR: cannot parse bundle: {exc}",
            file=sys.stderr,
        )
        sys.exit(2)


def load_trend() -> dict:
    if not TREND_PATH.exists():
        return {"cycles": []}
    try:
        data = json.loads(TREND_PATH.read_text())
        if not isinstance(data, dict) or "cycles" not in data:
            return {"cycles": []}
        return data
    except Exception:
        return {"cycles": []}


def save_trend(trend: dict) -> None:
    TREND_PATH.parent.mkdir(parents=True, exist_ok=True)
    TREND_PATH.write_text(json.dumps(trend, indent=2))


def failing_count_from_bundle(bundle: dict) -> int:
    """Return the count of product-critical failing Playwright IDs."""
    failing_ids: list = bundle.get("failing_ids", [])
    # Exclude non-Playwright IDs (e.g. API test IDs start with "[api/")
    pw_failing = [fid for fid in failing_ids if not fid.startswith("[api/")]
    return len(pw_failing)


def _two_consecutive_decreases(cycles: list[dict]) -> bool:
    """Return True if the last two consecutive transitions both decreased."""
    if len(cycles) < 3:
        return False
    c1, c2, c3 = cycles[-3], cycles[-2], cycles[-1]
    return c2["count"] < c1["count"] and c3["count"] < c2["count"]


def run(
    dry_run: bool = False,
    verbose: bool = False,
    status_only: bool = False,
) -> int:
    """Returns 0 (allowed), 1 (blocked), or 3 (dry-run blocked)."""
    bundle = load_bundle()
    trend = load_trend()
    cycles: list[dict] = trend.get("cycles", [])

    current_sha = bundle.get("commit_sha", "unknown")
    current_count = failing_count_from_bundle(bundle)
    current_ts = bundle.get("generated_at", int(time.time()))

    # Determine if this is a fresh cycle (new SHA).
    last_sha = cycles[-1]["sha"] if cycles else None
    is_fresh = current_sha != last_sha and current_sha != "unknown"

    if verbose or status_only:
        print(f"[meta-guard] bundle sha={current_sha} "
              f"failing={current_count}")
        print(f"[meta-guard] trend cycles={len(cycles)} "
              f"is_fresh={is_fresh}")
        for i, c in enumerate(cycles):
            print(f"  cycle[{i}]: sha={c['sha']} count={c['count']}")

    if status_only:
        allowed = (
            len(cycles) < 2
            or _two_consecutive_decreases(
                cycles + [{"sha": current_sha,
                           "count": current_count,
                           "ts": current_ts}]
            )
        )
        print(
            f"[meta-guard] status: "
            f"{'allowed' if allowed else 'blocked'}"
        )
        return 0

    # Append fresh cycle.
    if is_fresh and not dry_run:
        cycles.append({
            "sha": current_sha,
            "count": current_count,
            "ts": current_ts,
        })
        # Keep last MAX_CYCLES only.
        trend["cycles"] = cycles[-MAX_CYCLES:]
        save_trend(trend)
        if verbose:
            print(
                f"[meta-guard] recorded cycle: sha={current_sha} "
                f"count={current_count}"
            )

    # Evaluate trend with updated cycles.
    eval_cycles = trend["cycles"] if not dry_run else (
        cycles + (
            [{"sha": current_sha, "count": current_count,
              "ts": current_ts}]
            if is_fresh else []
        )
    )

    # Allow if insufficient history (< 3 data points) — gather data first.
    if len(eval_cycles) < 3:
        print(
            f"[meta-guard] ok: insufficient history "
            f"({len(eval_cycles)}/{MAX_CYCLES} cycles) — "
            f"meta tasks allowed"
        )
        return 0

    if _two_consecutive_decreases(eval_cycles):
        print(
            f"[meta-guard] ok: 2 consecutive decreases observed "
            f"({eval_cycles[-3]['count']} → "
            f"{eval_cycles[-2]['count']} → "
            f"{eval_cycles[-1]['count']}) — meta tasks allowed"
        )
        return 0

    # Blocked.
    counts = [c["count"] for c in eval_cycles[-3:]]
    msg = (
        f"[meta-guard] BLOCKED: no 2 consecutive decreases in "
        f"product-critical failing IDs over last "
        f"{len(eval_cycles)} cycles: {counts}\n"
        f"  Fix product-critical failing Playwright tests first.\n"
        f"  Current failing: {current_count}"
    )
    if dry_run:
        print(f"[meta-guard] dry-run: {msg}", file=sys.stderr)
        return 3
    print(msg, file=sys.stderr)
    return 1


def main() -> None:
    dry_run = "--dry-run" in sys.argv
    verbose = "--verbose" in sys.argv or "-v" in sys.argv
    status_only = "--status" in sys.argv

    code = run(dry_run=dry_run, verbose=verbose, status_only=status_only)
    sys.exit(code)


if __name__ == "__main__":
    main()
