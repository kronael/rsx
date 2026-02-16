"""
WebSocket stress test client for RSX Gateway
Integrated with playground API for real load testing
"""

import asyncio
import json
import time
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
    jwt_secret: str = "dev-secret-change-in-production"

    def __post_init__(self):
        if self.symbols is None:
            self.symbols = ["BTCUSD"]


@dataclass
class OrderMetrics:
    submitted: int = 0
    accepted: int = 0
    rejected: int = 0
    errors: int = 0
    latencies_us: list[int] = None

    def __post_init__(self):
        if self.latencies_us is None:
            self.latencies_us = []


class StressClient:
    """Single WebSocket connection worker"""

    def __init__(self, worker_id: int, config: StressConfig):
        self.worker_id = worker_id
        self.config = config
        self.user_id = (worker_id % config.users) + 1
        self.metrics = OrderMetrics()
        self.order_counter = 0

    def generate_jwt(self) -> str:
        """Generate JWT token for authentication"""
        payload = {
            "sub": str(self.user_id),
            "exp": int(time.time()) + 3600,
            "aud": "rsx-gateway",
            "iss": "rsx",
        }
        return pyjwt.encode(
            payload,
            self.config.jwt_secret,
            algorithm="HS256",
        )

    def _headers(self):
        """Auth headers for dev/testing"""
        return {"x-user-id": str(self.user_id)}

    def generate_order(self) -> str:
        """Generate compact wire-format order frame."""
        import random

        symbol_id = random.choice([1, 2, 3, 10])
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

    async def submit_order(self, ws) -> int | None:
        """Submit order and measure latency."""
        frame = self.generate_order()

        start = time.perf_counter_ns()
        await ws.send_str(frame)
        self.metrics.submitted += 1

        try:
            response = await asyncio.wait_for(
                ws.receive(timeout=1.0), timeout=1.0,
            )
            latency_ns = time.perf_counter_ns() - start

            if response.type == aiohttp.WSMsgType.TEXT:
                msg = json.loads(response.data)
                if "U" in msg:
                    self.metrics.accepted += 1
                    return latency_ns // 1000
                elif "E" in msg:
                    self.metrics.rejected += 1
                elif "H" in msg:
                    # Server heartbeat, not an order response
                    pass
                else:
                    self.metrics.errors += 1
            else:
                self.metrics.errors += 1

        except asyncio.TimeoutError:
            # No response = order pending (no Risk/ME)
            pass
        except Exception:
            self.metrics.errors += 1

        return None

    async def run_worker(self, rate_per_worker: float, duration: int):
        """Run stress test for this worker"""
        interval = 1.0 / rate_per_worker if rate_per_worker > 0 else 0.1

        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(
                    self.config.gateway_url,
                    headers=self._headers(),
                    timeout=aiohttp.ClientTimeout(total=5),
                ) as ws:
                    end_time = time.time() + duration

                    while time.time() < end_time:
                        loop_start = time.time()

                        latency = await self.submit_order(ws)

                        if latency is not None:
                            self.metrics.latencies_us.append(latency)

                        elapsed = time.time() - loop_start
                        sleep_time = max(0, interval - elapsed)
                        if sleep_time > 0:
                            await asyncio.sleep(sleep_time)

        except (aiohttp.ClientConnectorError, OSError) as e:
            self.metrics.errors += 1
            raise ConnectionError(
                f"cannot connect to gateway at "
                f"{self.config.gateway_url}: {e}"
            )
        except Exception as e:
            print(f"Worker {self.worker_id} error: {e}")
            raise


async def run_stress_test(config: StressConfig) -> dict:
    """Run multi-connection stress test"""

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

    # Check if all workers failed to connect
    conn_errors = [
        r for r in results_raw
        if isinstance(r, (ConnectionError, OSError))
    ]
    if conn_errors and len(conn_errors) == len(results_raw):
        return {
            "error": str(conn_errors[0]),
            "config": {
                "target_rate": config.rate,
                "duration": config.duration,
                "connections": config.connections,
            },
            "metrics": {
                "submitted": 0, "accepted": 0,
                "rejected": 0, "errors": len(conn_errors),
                "elapsed_sec": round(elapsed, 2),
                "actual_rate": 0.0, "accept_rate": 0.0,
            },
            "latency_us": {
                "p50": 0, "p95": 0, "p99": 0,
                "min": 0, "max": 0,
            },
        }

    # Aggregate metrics
    total_metrics = OrderMetrics()
    all_latencies = []

    for worker in workers:
        total_metrics.submitted += worker.metrics.submitted
        total_metrics.accepted += worker.metrics.accepted
        total_metrics.rejected += worker.metrics.rejected
        total_metrics.errors += worker.metrics.errors
        all_latencies.extend(worker.metrics.latencies_us)

    # Calculate percentiles
    all_latencies.sort()

    def percentile(data, p):
        if not data:
            return 0
        k = (len(data) - 1) * p / 100
        f = int(k)
        c = f + 1
        if c >= len(data):
            return data[-1]
        return data[f] + (k - f) * (data[c] - data[f])

    p50 = percentile(all_latencies, 50)
    p95 = percentile(all_latencies, 95)
    p99 = percentile(all_latencies, 99)

    actual_rate = total_metrics.submitted / elapsed

    results = {
        "config": {
            "target_rate": config.rate,
            "duration": config.duration,
            "connections": config.connections,
        },
        "metrics": {
            "submitted": total_metrics.submitted,
            "accepted": total_metrics.accepted,
            "rejected": total_metrics.rejected,
            "errors": total_metrics.errors,
            "elapsed_sec": round(elapsed, 2),
            "actual_rate": round(actual_rate, 2),
            "accept_rate": round(100 * total_metrics.accepted / max(total_metrics.submitted, 1), 2),
        },
        "latency_us": {
            "p50": round(p50),
            "p95": round(p95),
            "p99": round(p99),
            "min": all_latencies[0] if all_latencies else 0,
            "max": all_latencies[-1] if all_latencies else 0,
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
