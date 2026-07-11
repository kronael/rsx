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
    "RSX_STRESS_GW_URL", "ws://localhost:8088")
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
    ts = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
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
        f"p50={lat.get('p50')}us "
        f"p99={lat.get('p99')}us"
    )


def validate_results(results: dict, target_p99: int) -> list[str]:
    """Return correctness failures before evaluating latency."""
    if "error" in results:
        return [str(results["error"])]
    metrics = results.get("metrics")
    latency = results.get("latency_us")
    if not isinstance(metrics, dict) or not isinstance(latency, dict):
        return ["malformed stress result"]
    failures = []
    submitted = metrics.get("submitted", 0)
    accepted = metrics.get("accepted", 0)
    completed = metrics.get("completed", 0)
    timed_out = metrics.get("timed_out", 0)
    errors = metrics.get("errors", 0)
    pending = metrics.get("pending", 0)
    if accepted == 0 or completed == 0 or latency.get("samples", 0) == 0:
        failures.append("zero accepted/completed latency samples")
    if submitted != completed + timed_out + pending:
        failures.append("order accounting does not close")
    if completed != accepted + metrics.get("rejected", 0) + errors:
        failures.append("response accounting does not close")
    if metrics.get("offered", 0) != submitted + metrics.get("send_errors", 0):
        failures.append("offered accounting does not close")
    terminal = completed + timed_out
    if submitted and terminal / submitted < 0.95:
        failures.append("terminal outcomes below 95%")
    if pending:
        failures.append("unclassified pending orders remain")
    p99 = latency.get("p99")
    if p99 is None:
        failures.append("p99 unavailable")
    elif p99 >= target_p99:
        failures.append(f"p99={p99}us >= target={target_p99}us")
    return failures


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
                "offered": 0, "submitted": 0, "accepted": 0,
                "rejected": 0, "completed": 0, "timed_out": 0,
                "pending": 0, "errors": 0,
                "elapsed_sec": round(time.time() - start, 2),
                "actual_rate": 0.0, "accept_rate": 0.0,
            },
            "latency_us": {
                "samples": 0, "p50": None, "p95": None, "p99": None,
                "p99_9": None, "min": None, "max": None,
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

    failures = validate_results(results, TARGET_P99)
    results["status"] = "failed" if failures else "passed"
    results["failures"] = failures

    _print_stats(results)

    path = _save_report(results, REPORT_DIR)
    print(f"report: {path}")

    if not failures:
        p99 = results["latency_us"]["p99"]
        print(f"p99={p99}us < target={TARGET_P99}us: pass")
        return 0
    for failure in failures:
        print(f"{failure}: fail", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
