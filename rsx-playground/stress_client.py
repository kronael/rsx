"""
WebSocket stress test client for RSX Gateway
Integrated with playground API for real load testing
"""

import asyncio
import uuid
import json
import os
import time
from collections import Counter, deque
from dataclasses import dataclass
import aiohttp
from datetime import datetime
import jwt as pyjwt


@dataclass
class StressConfig:
    gateway_url: str = "ws://localhost:8080"
    rate: int = 1000  # orders per second
    duration: int = 60  # seconds
    symbols: list[str] = None
    users: int = 10
    connections: int = 10
    jwt_secret: str = ""

    def __post_init__(self):
        if self.symbols is None:
            self.symbols = ["BTCUSD"]


@dataclass
class OrderMetrics:
    offered: int = 0
    submitted: int = 0
    accepted: int = 0
    rejected: int = 0
    completed: int = 0
    timed_out: int = 0
    pending: int = 0
    errors: int = 0
    send_errors: int = 0
    rejected_by_reason: dict[str, int] = None
    latencies_us: list[int] = None

    def __post_init__(self):
        if self.latencies_us is None:
            self.latencies_us = []
        if self.rejected_by_reason is None:
            self.rejected_by_reason = {}


class StressClient:
    """Single WebSocket connection worker"""

    def __init__(self, worker_id: int, config: StressConfig):
        self.worker_id = worker_id
        self.config = config
        self.user_id = (worker_id % config.users) + 1
        # U/F responses carry server oid, not the submitted cid. Keeping each
        # connection on one ME preserves the order needed for the first
        # oid-to-cid binding while workers still spread load across symbols.
        self.symbol_id = (1, 2, 3, 10)[worker_id % 4]
        self.metrics = OrderMetrics()
        self.order_counter = 0

    def generate_jwt(self) -> str:
        """Generate JWT token for authentication"""
        secret = self.config.jwt_secret or os.environ.get(
            "RSX_GW_JWT_SECRET", "")
        if not secret:
            raise RuntimeError(
                "RSX_GW_JWT_SECRET not configured "
                "(pass --jwt-secret or set env var)")
        payload = {
            "sub": f"stress:{self.user_id}",
            "user_id": self.user_id,
            "exp": int(time.time()) + 3600,
            "aud": "rsx-gateway",
            "iss": "rsx-auth",
            "jti": uuid.uuid4().hex,
        }
        return pyjwt.encode(
            payload,
            secret,
            algorithm="HS256",
        )

    def _headers(self):
        """Auth headers for dev/testing"""
        return {
            "Authorization": f"Bearer {self.generate_jwt()}"
        }

    def generate_order(self) -> str:
        """Generate compact wire-format order frame."""
        import random

        symbol_id = self.symbol_id
        side = random.choice([0, 1])

        # Fixed-point i64 price in tick units
        price = 50000 + random.randint(-100, 100)
        qty = random.randint(1, 100)
        tif = 0  # GTC

        self.order_counter += 1
        # cid must be exactly 20 chars, zero-padded
        cid_raw = f"s{self.worker_id}-{self.order_counter}"
        cid = cid_raw[:20].ljust(20, "0")

        return json.dumps({
            "N": [
                symbol_id,
                side,
                price,
                qty,
                cid,
                tif,
            ]
        })

    @staticmethod
    def _client_order_id(frame: str) -> str:
        """Return the cid carried by a generated N frame."""
        return str(json.loads(frame)["N"][4])

    def _handle_response(
        self,
        msg: dict,
        awaiting: deque[str],
        pending_by_cid: dict[str, int],
        oid_to_cid: dict[str, str],
    ) -> None:
        """Account for one gateway frame using protocol order identity.

        New-order replies do not echo cid.  The first frame carrying a new
        server oid binds it to the oldest unbound cid; subsequent fills and
        updates are routed by oid.  Fills are deliberately non-terminal -- a
        possibly interleaved terminal U owns the latency sample.
        """
        if "H" in msg:
            return

        if "F" in msg:
            fill = msg.get("F", [])
            if len(fill) < 2:
                self.metrics.errors += 1
                return
            # A taker's first frame may be F (fills precede ORDER_DONE).
            taker_oid = str(fill[0])
            if taker_oid not in oid_to_cid and awaiting:
                oid_to_cid[taker_oid] = awaiting.popleft()
            return

        if "E" in msg:
            if not awaiting:
                self.metrics.errors += 1
                return
            cid = awaiting.popleft()
            if pending_by_cid.pop(cid, None) is None:
                self.metrics.errors += 1
                return
            reason = self._reject_reason(msg) or "protocol_error"
            counts = Counter(self.metrics.rejected_by_reason)
            counts[reason] += 1
            self.metrics.rejected_by_reason = dict(counts)
            self.metrics.rejected += 1
            self.metrics.completed += 1
            return

        if "U" not in msg:
            self.metrics.errors += 1
            return

        update = msg.get("U", [])
        if len(update) < 2:
            self.metrics.errors += 1
            return
        oid = str(update[0])
        cid = oid_to_cid.get(oid)
        if cid is None and awaiting:
            cid = awaiting.popleft()
            oid_to_cid[oid] = cid
        if cid is None:
            self.metrics.errors += 1
            return
        sent_ns = pending_by_cid.pop(cid, None)
        if sent_ns is None:
            self.metrics.errors += 1
            return

        reason = self._reject_reason(msg)
        if reason is not None:
            self.metrics.rejected += 1
            counts = Counter(self.metrics.rejected_by_reason)
            counts[reason] += 1
            self.metrics.rejected_by_reason = dict(counts)
        else:
            self.metrics.accepted += 1
            self.metrics.latencies_us.append(
                (time.perf_counter_ns() - sent_ns) // 1000)
        self.metrics.completed += 1

    @staticmethod
    def _reject_reason(msg: dict) -> str | None:
        if "E" in msg:
            error = msg["E"]
            return str(error[0] if error else "protocol_error")
        update = msg.get("U", [])
        if len(update) > 1 and update[1] == 3:
            return str(update[4] if len(update) > 4 else "unknown")
        return None

    async def run_worker(self, rate_per_worker: float, duration: int):
        """Send open-loop at the configured rate and drain responses."""
        interval = 1.0 / rate_per_worker if rate_per_worker > 0 else 0.1

        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(
                    self.config.gateway_url,
                    headers=self._headers(),
                    timeout=aiohttp.ClientTimeout(total=5),
                ) as ws:
                    loop = asyncio.get_running_loop()
                    awaiting = deque()
                    pending_by_cid: dict[str, int] = {}
                    oid_to_cid: dict[str, str] = {}
                    send_done = asyncio.Event()

                    async def sender():
                        started = loop.time()
                        next_send = started
                        while loop.time() - started < duration:
                            if next_send - started >= duration:
                                break
                            delay = next_send - loop.time()
                            if delay > 0:
                                await asyncio.sleep(delay)
                            self.metrics.offered += 1
                            try:
                                frame = self.generate_order()
                                cid = self._client_order_id(frame)
                                await ws.send_str(frame)
                            except Exception:
                                self.metrics.send_errors += 1
                                next_send += interval
                                continue
                            self.metrics.submitted += 1
                            pending_by_cid[cid] = time.perf_counter_ns()
                            awaiting.append(cid)
                            next_send += interval
                        send_done.set()

                    async def receiver():
                        drain_deadline = None
                        while True:
                            if send_done.is_set() and drain_deadline is None:
                                drain_deadline = loop.time() + 1.0
                            if send_done.is_set() and not pending_by_cid:
                                return
                            if drain_deadline is not None and loop.time() >= drain_deadline:
                                return
                            timeout = 0.1
                            if drain_deadline is not None:
                                timeout = min(
                                    timeout,
                                    max(0.001, drain_deadline - loop.time()),
                                )
                            try:
                                response = await ws.receive(timeout=timeout)
                            except asyncio.TimeoutError:
                                continue
                            if response.type != aiohttp.WSMsgType.TEXT:
                                self.metrics.errors += 1
                                continue
                            try:
                                msg = json.loads(response.data)
                            except (TypeError, json.JSONDecodeError):
                                self.metrics.errors += 1
                                continue
                            self._handle_response(
                                msg, awaiting, pending_by_cid, oid_to_cid)

                    await asyncio.gather(sender(), receiver())
                    self.metrics.timed_out += len(pending_by_cid)
                    pending_by_cid.clear()

        except (
            aiohttp.ClientConnectorError,
            aiohttp.ServerDisconnectedError,
            aiohttp.ClientConnectionError,
            OSError,
        ) as e:
            self.metrics.errors += 1
            raise ConnectionError(
                f"gateway unreachable at "
                f"{self.config.gateway_url}: {e}"
            )
        except Exception as e:
            print(f"Worker {self.worker_id} error: {e}")
            raise


async def _probe_gateway(
    url: str,
    jwt_secret: str = "",
) -> str | None:
    """Return error string if gateway unreachable, else None."""
    secret = jwt_secret or os.environ.get("RSX_GW_JWT_SECRET", "")
    if not secret:
        return "RSX_GW_JWT_SECRET not configured"
    token = pyjwt.encode(
        {
            "sub": "stress:1",
            "user_id": 1,
            "exp": int(time.time()) + 3600,
            "aud": "rsx-gateway",
            "iss": "rsx-auth",
            "jti": uuid.uuid4().hex,
        },
        secret,
        algorithm="HS256",
    )
    headers = {"Authorization": f"Bearer {token}"}
    try:
        async with aiohttp.ClientSession() as session:
            async with session.ws_connect(
                url,
                headers=headers,
                timeout=aiohttp.ClientTimeout(total=3),
            ):
                pass
        return None
    except (
        aiohttp.ClientConnectorError,
        aiohttp.ServerDisconnectedError,
        aiohttp.ClientConnectionError,
        OSError,
    ) as e:
        return f"gateway unreachable at {url}: {e}"
    except Exception:
        return None  # connected but other error — gateway is up


async def run_stress_test(config: StressConfig) -> dict:
    """Run multi-connection stress test"""

    # Fail fast: probe gateway before spinning up workers
    err = await _probe_gateway(
        config.gateway_url, config.jwt_secret
    )
    if err:
        return {
            "error": err,
            "config": {
                "target_rate": config.rate,
                "duration": config.duration,
                "connections": config.connections,
            },
            "metrics": {
                "offered": 0, "submitted": 0, "accepted": 0,
                "rejected": 0, "completed": 0, "timed_out": 0,
                "pending": 0, "errors": 1, "rejected_by_reason": {},
                "send_errors": 0,
                "elapsed_sec": 0.0,
                "actual_rate": 0.0, "accept_rate": 0.0,
            },
            "latency_us": {
                "samples": 0, "p50": None, "p95": None, "p99": None,
                "p99_9": None, "min": None, "max": None,
            },
        }

    rate_per_worker = config.rate / config.connections

    # Create workers
    workers = [
        StressClient(i, config)
        for i in range(config.connections)
    ]

    # Run all workers concurrently
    tasks = [
        worker.run_worker(rate_per_worker, config.duration)
        for worker in workers
    ]

    print(f"Starting {config.connections} workers at "
          f"{rate_per_worker:.1f} orders/sec each")
    print(f"Target: {config.rate} orders/sec for "
          f"{config.duration} seconds")

    start_time = time.time()
    results_raw = await asyncio.gather(
        *tasks, return_exceptions=True,
    )
    elapsed = time.time() - start_time

    # Check if all workers failed (any exception type)
    all_errors = [
        r for r in results_raw if isinstance(r, BaseException)
    ]
    if all_errors and len(all_errors) == len(results_raw):
        return {
            "error": str(all_errors[0]),
            "config": {
                "target_rate": config.rate,
                "duration": config.duration,
                "connections": config.connections,
            },
            "metrics": {
                "offered": 0, "submitted": 0, "accepted": 0,
                "rejected": 0, "completed": 0, "timed_out": 0,
                "pending": 0, "errors": len(all_errors),
                "rejected_by_reason": {},
                "send_errors": 0,
                "elapsed_sec": round(elapsed, 2),
                "actual_rate": 0.0, "accept_rate": 0.0,
            },
            "latency_us": {
                "samples": 0, "p50": None, "p95": None, "p99": None,
                "p99_9": None, "min": None, "max": None,
            },
        }

    # Aggregate metrics
    total_metrics = OrderMetrics()
    all_latencies = []

    for worker in workers:
        total_metrics.offered += worker.metrics.offered
        total_metrics.submitted += worker.metrics.submitted
        total_metrics.accepted += worker.metrics.accepted
        total_metrics.rejected += worker.metrics.rejected
        total_metrics.completed += worker.metrics.completed
        total_metrics.timed_out += worker.metrics.timed_out
        total_metrics.pending += worker.metrics.pending
        total_metrics.errors += worker.metrics.errors
        total_metrics.send_errors += worker.metrics.send_errors
        counts = Counter(total_metrics.rejected_by_reason)
        counts.update(worker.metrics.rejected_by_reason)
        total_metrics.rejected_by_reason = dict(counts)
        all_latencies.extend(worker.metrics.latencies_us)

    # Calculate percentiles
    all_latencies.sort()

    def percentile(data, p):
        if not data:
            return None
        k = (len(data) - 1) * p / 100
        f = int(k)
        c = f + 1
        if c >= len(data):
            return data[-1]
        return data[f] + (k - f) * (data[c] - data[f])

    p50 = percentile(all_latencies, 50)
    p95 = percentile(all_latencies, 95)
    p99 = percentile(all_latencies, 99)
    p99_9 = percentile(all_latencies, 99.9)

    actual_rate = total_metrics.submitted / elapsed

    results = {
        "config": {
            "target_rate": config.rate,
            "duration": config.duration,
            "connections": config.connections,
        },
        "metrics": {
            "offered": total_metrics.offered,
            "submitted": total_metrics.submitted,
            "accepted": total_metrics.accepted,
            "rejected": total_metrics.rejected,
            "rejected_by_reason": total_metrics.rejected_by_reason,
            "completed": total_metrics.completed,
            "timed_out": total_metrics.timed_out,
            "pending": total_metrics.pending,
            "errors": total_metrics.errors,
            "send_errors": total_metrics.send_errors,
            "elapsed_sec": round(elapsed, 2),
            "actual_rate": round(actual_rate, 2),
            "achieved_rate": round(total_metrics.completed / elapsed, 2),
            "accept_rate": round(
                100 * total_metrics.accepted
                / max(total_metrics.submitted, 1),
                2,
            ),
        },
        "latency_us": {
            "samples": len(all_latencies),
            "p50": round(p50) if p50 is not None else None,
            "p95": round(p95) if p95 is not None else None,
            "p99": round(p99) if p99 is not None else None,
            "p99_9": round(p99_9) if p99_9 is not None else None,
            "min": all_latencies[0] if all_latencies else None,
            "max": all_latencies[-1] if all_latencies else None,
        }
    }

    return results


if __name__ == "__main__":
    import sys

    config = StressConfig(
        rate=int(sys.argv[1]) if len(sys.argv) > 1 else 1000,
        duration=int(sys.argv[2]) if len(sys.argv) > 2 else 60,
    )

    results = asyncio.run(run_stress_test(config))

    print("\n=== Stress Test Results ===")
    print(f"Submitted: {results['metrics']['submitted']}")
    print(f"Accepted: {results['metrics']['accepted']} ({results['metrics']['accept_rate']}%)")
    print(f"Rejected: {results['metrics']['rejected']}")
    print(f"Errors: {results['metrics']['errors']}")
    print(f"Actual rate: {results['metrics']['actual_rate']} orders/sec")
    print(f"\nLatency (us):")
    print(f"  p50: {results['latency_us']['p50']}")
    print(f"  p95: {results['latency_us']['p95']}")
    print(f"  p99: {results['latency_us']['p99']}")
