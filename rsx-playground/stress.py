"""
RSX stress test entry point.

Reads config from env vars, calls stress_client.run_stress_test,
prints periodic stats every 5s, writes JSON report to REPORT_DIR/,
exits 0 if p99 < TARGET_P99 else 1.

SIGTERM: cancel workers, write partial report.
"""

import asyncio
import json
import os
import signal
import sys
import time
from datetime import datetime
from pathlib import Path

from stress_client import StressConfig
from stress_client import run_stress_test


GW_URL = os.environ.get(
    "RSX_STRESS_GW_URL", "ws://localhost:8080")
USERS = int(os.environ.get("RSX_STRESS_USERS", "10"))
RATE = int(os.environ.get("RSX_STRESS_RATE", "1000"))
DURATION = int(os.environ.get("RSX_STRESS_DURATION", "60"))
TARGET_P99 = int(os.environ.get("RSX_STRESS_TARGET_P99", "50000"))
REPORT_DIR = Path(
    os.environ.get("RSX_STRESS_REPORT_DIR", "./tmp/stress"))

_cancelled = False


def _handle_sigterm(signum, frame):
    global _cancelled
    _cancelled = True


signal.signal(signal.SIGTERM, _handle_sigterm)
signal.signal(signal.SIGINT, _handle_sigterm)


def _save_report(results: dict, report_dir: Path) -> Path:
    report_dir.mkdir(parents=True, exist_ok=True)
    ts = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = report_dir / f"stress-{ts}.json"
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(results, indent=2))
    tmp.replace(path)
    return path


def _print_stats(results: dict) -> None:
    m = results.get("metrics", {})
    lat = results.get("latency_us", {})
    ts = datetime.now().strftime("%b %d %H:%M:%S")
    print(
        f"{ts} submitted={m.get('submitted', 0)} "
        f"accepted={m.get('accepted', 0)} "
        f"rate={m.get('actual_rate', 0)}/s "
        f"p50={lat.get('p50', 0)}us "
        f"p99={lat.get('p99', 0)}us"
    )


async def _run_with_stats(config: StressConfig) -> dict:
    """Run stress test; print stats during run (best-effort)."""
    global _cancelled

    # Periodic stats: print a line every 5s while test runs
    stats_interval = 5.0
    start = time.time()

    task = asyncio.create_task(run_stress_test(config))

    while not task.done():
        try:
            await asyncio.wait_for(
                asyncio.shield(task), timeout=stats_interval)
        except asyncio.TimeoutError:
            elapsed = time.time() - start
            ts = datetime.now().strftime("%b %d %H:%M:%S")
            print(
                f"{ts} running... elapsed={elapsed:.0f}s "
                f"(target {config.duration}s)"
            )
        except asyncio.CancelledError:
            task.cancel()
            break

        if _cancelled:
            task.cancel()
            break

    try:
        results = await task
    except asyncio.CancelledError:
        results = {
            "partial": True,
            "config": {
                "target_rate": config.rate,
                "duration": config.duration,
                "connections": config.connections,
            },
            "metrics": {
                "submitted": 0, "accepted": 0,
                "rejected": 0, "errors": 0,
                "elapsed_sec": round(time.time() - start, 2),
                "actual_rate": 0.0, "accept_rate": 0.0,
            },
            "latency_us": {
                "p50": 0, "p95": 0, "p99": 0,
                "min": 0, "max": 0,
            },
        }

    return results


async def main() -> int:
    config = StressConfig(
        gateway_url=GW_URL,
        users=USERS,
        connections=USERS,
        rate=RATE,
        duration=DURATION,
    )

    ts = datetime.now().strftime("%b %d %H:%M:%S")
    print(
        f"{ts} starting stress: gw={GW_URL} "
        f"users={USERS} rate={RATE}/s duration={DURATION}s "
        f"target_p99={TARGET_P99}us"
    )

    results = await _run_with_stats(config)

    _print_stats(results)

    path = _save_report(results, REPORT_DIR)
    print(f"report: {path}")

    if "error" in results:
        print(f"error: {results['error']}", file=sys.stderr)
        return 1

    p99 = results.get("latency_us", {}).get("p99", 0)
    if p99 < TARGET_P99:
        print(f"p99={p99}us < target={TARGET_P99}us: pass")
        return 0
    else:
        print(
            f"p99={p99}us >= target={TARGET_P99}us: fail",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
