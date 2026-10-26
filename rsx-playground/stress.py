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
import urllib.request
from concurrent.futures import ProcessPoolExecutor
from datetime import datetime
from pathlib import Path

import notional
from stress_client import StressConfig
from stress_client import _probe_gateway
from stress_client import run_stress_test


GW_URL = os.environ.get(
    "RSX_STRESS_GW_URL", "ws://localhost:8088")
USERS = int(os.environ.get("RSX_STRESS_USERS", "10"))
CONNECTIONS = int(os.environ.get("RSX_STRESS_CONNECTIONS", str(USERS)))
PROCESSES = int(os.environ.get("RSX_STRESS_PROCESSES", "1"))
SYMBOLS = [
    int(value)
    for value in os.environ.get("RSX_STRESS_SYMBOLS", "10").split(",")
]
SYMBOL_CONFIGS = {
    int(symbol_id): values
    for symbol_id, values in json.loads(
        os.environ.get("RSX_STRESS_SYMBOL_CONFIGS", "{}")
    ).items()
}
LOT_SIZE = int(os.environ.get("RSX_STRESS_LOT_SIZE", "100000"))
# Minimum order notional (raw price x qty units): symbol configs and the
# catalog at SYMBOLS_URL carry it per symbol; MIN_NOTIONAL overrides the
# single-symbol fallback.
MIN_NOTIONAL = int(os.environ.get("RSX_STRESS_MIN_NOTIONAL", "0"))
SYMBOLS_URL = os.environ.get(
    "RSX_STRESS_SYMBOLS_URL", "http://127.0.0.1:49171/v1/symbols")
BID_PRICE = int(os.environ.get("RSX_STRESS_BID_PRICE", "49000"))
ASK_PRICE = int(os.environ.get("RSX_STRESS_ASK_PRICE", "51000"))
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


def fetch_catalog_minimums(url: str) -> dict[int, int]:
    """symbol_id -> minimum order notional from a `/v1/symbols` catalog;
    empty when the catalog is unreachable."""
    try:
        with urllib.request.urlopen(url, timeout=3) as response:
            rows = json.load(response)["symbols"]
        return {
            int(row["id"]): notional.catalog_min_notional(row)
            for row in rows
        }
    except (OSError, ValueError, KeyError, TypeError) as exc:
        print(f"stress: no minimum order size from {url}: {exc}; "
              "orders are one lot each", file=sys.stderr)
        return {}


def resolve_minimums(
    symbols: list[int],
    symbol_configs: dict[int, dict],
    fixed: int,
    catalog: dict[int, int],
) -> tuple[dict[int, dict], int]:
    """Fill each symbol config's `min_notional` from the catalog and pick
    the fallback for runs without symbol configs. A value already in a
    config wins."""
    resolved = {
        symbol_id: {
            **values,
            "min_notional": values.get("min_notional")
            or catalog.get(symbol_id, 0),
        }
        for symbol_id, values in symbol_configs.items()
    }
    return resolved, fixed or catalog.get(symbols[0], 0)


def _save_report(results: dict, report_dir: Path) -> Path:
    report_dir.mkdir(parents=True, exist_ok=True)
    ts = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    path = report_dir / f"stress-{ts}.json"
    tmp = path.with_suffix(".tmp")
    def public(value):
        if isinstance(value, dict):
            return {
                key: public(item) for key, item in value.items()
                if not key.startswith("_")
            }
        if isinstance(value, list):
            return [public(item) for item in value]
        return value

    public_results = public(results)
    tmp.write_text(json.dumps(public_results, indent=2))
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
    fill_attempts = metrics.get("fill_attempts")
    filled_orders = metrics.get("filled_orders")
    fills = metrics.get("fills")
    if fill_attempts is not None and fill_attempts != submitted:
        failures.append("fill attempts do not equal submitted orders")
    if filled_orders is not None and filled_orders != accepted:
        failures.append("accepted crossing orders did not all fill")
    if (fills is not None and filled_orders is not None
            and fills < filled_orders):
        failures.append("fill frames below filled-order count")
    timestamp_errors = metrics.get("timestamp_errors", {})
    if not isinstance(timestamp_errors, dict):
        failures.append("malformed timestamp errors")
    elif any(
        not isinstance(count, int) or isinstance(count, bool) or count < 0
        for count in timestamp_errors.values()
    ):
        failures.append("malformed timestamp errors")
    elif any(timestamp_errors.values()):
        failures.append("invalid server timestamps: " + ", ".join(
            f"{name}={count}" for name, count in sorted(timestamp_errors.items())
            if count
        ))
    server = results.get("server_latency_ns", {})
    for leg in ("internal", "gateway_to_risk", "match_done_to_gateway"):
        if server.get(leg, {}).get("samples") != accepted:
            failures.append(f"{leg} samples do not equal accepted")
    for leg in ("risk_to_me", "matching"):
        if server.get(leg, {}).get("samples") != filled_orders:
            failures.append(f"{leg} samples do not equal filled orders")
    terminal = completed + timed_out
    if submitted and terminal / submitted < 0.95:
        failures.append("terminal outcomes below 95%")
    if pending:
        failures.append("unclassified pending orders remain")
    per_symbol = results.get("per_symbol", {})
    configured = results.get("config", {}).get("symbols", [])
    names = ("offered", "submitted", "accepted", "rejected", "completed",
             "timed_out", "pending", "errors", "send_errors",
             "fill_attempts", "filled_orders", "fills")
    for symbol in configured:
        row = per_symbol.get(str(symbol), {})
        symbol_metrics = row.get("metrics", {})
        if not symbol_metrics:
            failures.append(f"symbol {symbol} result missing")
            continue
        symbol_server = row.get("server_latency_ns", {})
        expected = {
            "internal": symbol_metrics.get("accepted", 0),
            "gateway_to_risk": symbol_metrics.get("accepted", 0),
            "risk_to_me": symbol_metrics.get("filled_orders", 0),
            "matching": symbol_metrics.get("filled_orders", 0),
            "match_done_to_gateway": symbol_metrics.get("accepted", 0),
        }
        for leg, count in expected.items():
            if symbol_server.get(leg, {}).get("samples") != count:
                failures.append(
                    f"symbol {symbol} {leg} samples do not close")
    for name in names:
        if configured and sum(
            per_symbol.get(str(symbol), {}).get("metrics", {}).get(name, 0)
            for symbol in configured
        ) != metrics.get(name, 0):
            failures.append(f"per-symbol {name} does not close")
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
                "users": config.users,
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


def _run_partition(config: StressConfig) -> dict:
    return asyncio.run(run_stress_test(config))


def _percentile(values: list[int], percentile: float):
    if not values:
        return None
    values.sort()
    position = (len(values) - 1) * percentile / 100
    lower = int(position)
    upper = min(lower + 1, len(values) - 1)
    return values[lower] + (position - lower) * (values[upper] - values[lower])


def _latency_summary(values: list[int], include_tail: bool = False) -> dict:
    summary = {
        "samples": len(values),
        "p50": round(_percentile(values, 50)) if values else None,
        "p95": round(_percentile(values, 95)) if values else None,
        "p99": round(_percentile(values, 99)) if values else None,
        "max": max(values) if values else None,
    }
    if include_tail:
        summary["p99_9"] = round(_percentile(values, 99.9)) if values else None
        summary["min"] = min(values) if values else None
    return summary


def _merge_partitions(results: list[dict], config: StressConfig) -> dict:
    metrics = {
        key: sum(result["metrics"].get(key, 0) for result in results)
        for key in ("offered", "submitted", "accepted", "rejected",
                    "completed", "timed_out", "pending", "errors",
                    "send_errors", "fill_attempts", "filled_orders", "fills")
    }
    for key in ("rejected_by_reason", "errors_by_reason"):
        counts = {}
        for result in results:
            for reason, count in result["metrics"].get(key, {}).items():
                counts[reason] = counts.get(reason, 0) + count
        metrics[key] = counts
    timestamp_errors = {}
    for result in results:
        for reason, count in result["metrics"].get(
            "timestamp_errors", {}
        ).items():
            timestamp_errors[reason] = timestamp_errors.get(reason, 0) + count
    metrics["timestamp_errors"] = timestamp_errors
    elapsed = max(result["metrics"].get("elapsed_sec", 0) for result in results)
    metrics.update({
        "elapsed_sec": elapsed,
        "actual_rate": round(metrics["submitted"] / elapsed, 2) if elapsed else 0,
        "achieved_rate": round(metrics["completed"] / elapsed, 2) if elapsed else 0,
        "accept_rate": round(100 * metrics["accepted"] /
                             max(metrics["submitted"], 1), 2),
    })
    client = [sample for result in results
              for sample in result.get("_latency_samples_us", [])]
    leg_names = ("internal", "gateway_to_risk", "risk_to_me", "matching",
                 "match_done_to_gateway")
    legs = {
        leg: [sample for result in results
              for sample in result.get("_server_latency_samples_ns", {})
              .get(leg, [])]
        for leg in leg_names
    }
    merged = {
        "config": {"target_rate": config.rate, "duration": config.duration,
                   "users": config.users, "connections": config.connections,
                   "processes": len(results),
                   "symbols": config.symbols,
                   "bid_price": config.bid_price,
                   "ask_price": config.ask_price},
        "metrics": metrics,
        "latency_us": _latency_summary(client, include_tail=True),
        "server_latency_ns": {
            leg: _latency_summary(samples) for leg, samples in legs.items()
        },
        "_latency_samples_us": client,
        "_server_latency_samples_ns": legs,
    }
    per_symbol = {}
    for symbol_id in config.symbols:
        key = str(symbol_id)
        parts = [result.get("per_symbol", {}).get(key, {})
                 for result in results]
        symbol_metrics = {
            name: sum(part.get("metrics", {}).get(name, 0)
                      for part in parts)
            for name in (
                "offered", "submitted", "accepted", "rejected",
                "completed", "timed_out", "pending", "errors",
                "send_errors", "fill_attempts", "filled_orders", "fills",
            )
        }
        symbol_client = [
            sample for part in parts
            for sample in part.get("_latency_samples_us", [])
        ]
        symbol_legs = {
            leg: [
                sample for part in parts
                for sample in part.get("_server_latency_samples_ns", {})
                .get(leg, [])
            ]
            for leg in leg_names
        }
        per_symbol[key] = {
            "metrics": symbol_metrics,
            "latency_us": _latency_summary(symbol_client),
            "server_latency_ns": {
                leg: _latency_summary(samples)
                for leg, samples in symbol_legs.items()
            },
            "_latency_samples_us": symbol_client,
            "_server_latency_samples_ns": symbol_legs,
        }
    merged["per_symbol"] = per_symbol
    return merged


def _partition_configs(
    config: StressConfig,
    processes: int,
    start_at_ns: int,
) -> list[StressConfig]:
    count = min(processes, config.connections, config.rate)
    base, extra = divmod(config.connections, count)
    partitions = []
    offset = 0
    for index in range(count):
        connections = base + (1 if index < extra else 0)
        rate = config.rate * connections // config.connections
        if index == count - 1:
            rate = config.rate - sum(part.rate for part in partitions)
        partitions.append(StressConfig(
            gateway_url=config.gateway_url, rate=rate,
            duration=config.duration, symbols=config.symbols,
            users=config.users, connections=connections,
            lot_size=config.lot_size, jwt_secret=config.jwt_secret,
            bid_price=config.bid_price, ask_price=config.ask_price,
            symbol_configs=config.symbol_configs,
            min_notional=config.min_notional,
            worker_offset=offset,
            start_at_ns=start_at_ns,
            quiet=True,
            probe_gateway=False,
        ))
        offset += connections
    return partitions


async def _run_processes(config: StressConfig, processes: int) -> dict:
    error = await _probe_gateway(config.gateway_url, config.jwt_secret)
    if error:
        return {"error": error}
    partitions = _partition_configs(
        config,
        processes,
        time.time_ns() + 5_000_000_000,
    )
    count = len(partitions)
    loop = asyncio.get_running_loop()
    with ProcessPoolExecutor(max_workers=count) as pool:
        results = await asyncio.gather(*[
            loop.run_in_executor(pool, _run_partition, partition)
            for partition in partitions
        ])
    return _merge_partitions(results, config)


async def main() -> int:
    symbol_configs, min_notional = resolve_minimums(
        SYMBOLS, SYMBOL_CONFIGS, MIN_NOTIONAL,
        fetch_catalog_minimums(SYMBOLS_URL))
    config = StressConfig(
        gateway_url=GW_URL,
        users=USERS,
        connections=CONNECTIONS,
        symbols=SYMBOLS,
        lot_size=LOT_SIZE,
        bid_price=BID_PRICE,
        ask_price=ASK_PRICE,
        symbol_configs=symbol_configs,
        min_notional=min_notional,
        rate=RATE,
        duration=DURATION,
    )

    ts = datetime.now().strftime("%b %d %H:%M:%S")
    print(
        f"{ts} starting stress: gw={GW_URL} "
        f"users={USERS} connections={CONNECTIONS} "
        f"rate={RATE}/s duration={DURATION}s "
        f"target_p99={TARGET_P99}us"
    )

    if PROCESSES > 1:
        results = await _run_processes(config, PROCESSES)
    else:
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
