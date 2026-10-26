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

import notional
import order_ids


@dataclass
class StressConfig:
    gateway_url: str = "ws://localhost:8080"
    rate: int = 1000  # orders per second
    duration: int = 60  # seconds
    symbols: list[int] = None
    users: int = 10
    connections: int = 10
    lot_size: int = 100_000
    bid_price: int = 49_000
    ask_price: int = 51_000
    symbol_configs: dict[int, dict[str, int]] = None
    # Minimum order notional in risk's raw price x qty unit, for the
    # single-symbol fallback prices; a symbol config carries its own
    # `min_notional`. 0 = no floor known, one lot per order.
    min_notional: int = 0
    jwt_secret: str = ""
    worker_offset: int = 0
    start_at_ns: int = 0
    quiet: bool = False
    probe_gateway: bool = True

    def __post_init__(self):
        if self.symbols is None:
            self.symbols = [10]
        if self.symbol_configs is None:
            self.symbol_configs = {}
        if self.symbol_configs:
            missing = set(self.symbols) - self.symbol_configs.keys()
            if missing:
                raise ValueError(f"missing symbol configs: {sorted(missing)}")
            for symbol_id, values in self.symbol_configs.items():
                tick = values.get("tick_size", 0)
                lot = values.get("lot_size", 0)
                bid = values.get("bid_price", 0)
                ask = values.get("ask_price", 0)
                if tick <= 0 or lot <= 0 or bid <= 0 or ask <= bid:
                    raise ValueError(f"invalid symbol config: {symbol_id}")
                if bid % tick or ask % tick:
                    raise ValueError(f"unaligned symbol BBO: {symbol_id}")
                if values.get("min_notional", 0) < 0:
                    raise ValueError(
                        f"negative min_notional: {symbol_id}")
        if self.lot_size <= 0:
            raise ValueError("lot_size must be positive")
        if self.min_notional < 0:
            raise ValueError("min_notional must not be negative")
        if self.bid_price <= 0 or self.ask_price <= self.bid_price:
            raise ValueError("crossing prices require 0 < bid < ask")


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
    elapsed_sec: float = 0.0
    rejected_by_reason: dict[str, int] = None
    errors_by_reason: dict[str, int] = None
    latencies_us: list[int] = None
    server_latencies_ns: dict[str, list[int]] = None
    timestamp_errors: dict[str, int] = None
    fill_attempts: int = 0
    filled_orders: int = 0
    fills: int = 0

    def __post_init__(self):
        if self.latencies_us is None:
            self.latencies_us = []
        if self.server_latencies_ns is None:
            self.server_latencies_ns = {
                "internal": [],
                "gateway_to_risk": [],
                "risk_to_me": [],
                "matching": [],
                "match_done_to_gateway": [],
            }
        if self.timestamp_errors is None:
            self.timestamp_errors = {}
        if self.rejected_by_reason is None:
            self.rejected_by_reason = {}
        if self.errors_by_reason is None:
            self.errors_by_reason = {}


def _record_error(metrics: OrderMetrics, reason: str) -> None:
    metrics.errors += 1
    counts = Counter(metrics.errors_by_reason)
    counts[reason] += 1
    metrics.errors_by_reason = dict(counts)


class StressClient:
    """Single WebSocket connection worker"""

    def __init__(self, worker_id: int, config: StressConfig):
        self.worker_id = worker_id + config.worker_offset
        self.pacing_index = worker_id
        self.config = config
        self.user_id = (self.worker_id % config.users) + 1
        if self.user_id >= 99:
            self.user_id += 1
        # U/F responses carry server oid, not the submitted cid. Keeping each
        # connection on one ME preserves the order needed for the first
        # oid-to-cid binding while configured symbols can still spread load.
        self.symbol_id = config.symbols[self.worker_id % len(config.symbols)]
        self.symbol_config = config.symbol_configs.get(self.symbol_id, {})
        self.metrics = OrderMetrics()
        self.order_counter = 0
        self.cid_mint = order_ids.CidMint()
        self.filled_oids: set[str] = set()

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
        """Generate a deterministic crossing IOC order."""
        symbol_id = self.symbol_id
        side = self.order_counter % 2
        price = self.symbol_config.get(
            "ask_price" if side == 0 else "bid_price",
            self.config.ask_price if side == 0 else self.config.bid_price,
        )
        lot = self.symbol_config.get("lot_size", self.config.lot_size)
        # Risk rejects an order whose price x qty is under the symbol's
        # minimum; the order is sized to reach it at its own price.
        qty = notional.min_order_qty(
            self.symbol_config.get(
                "min_notional", self.config.min_notional),
            price, lot)
        tif = 1  # IOC keeps repeated load runs from accumulating open orders

        self.order_counter += 1
        cid = self.cid_mint.next()

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
        server_legs_by_oid: dict[str, list[int]] | None = None,
    ) -> None:
        """Account for one gateway frame using protocol order identity.

        New-order replies do not echo cid.  The first frame carrying a new
        server oid binds it to the oldest unbound cid; subsequent fills and
        updates are routed by oid.  Fills are deliberately non-terminal -- a
        possibly interleaved terminal U owns the latency sample.
        """
        if server_legs_by_oid is None:
            server_legs_by_oid = {}
        if "H" in msg:
            return

        if "F" in msg:
            fill = msg.get("F", [])
            if len(fill) < 2:
                _record_error(self.metrics, "short_fill")
                return
            # A taker's first frame may be F (fills precede ORDER_DONE).
            taker_oid = str(fill[0])
            self.metrics.fills += 1
            if taker_oid not in self.filled_oids:
                self.filled_oids.add(taker_oid)
                self.metrics.filled_orders += 1
            if taker_oid not in oid_to_cid and awaiting:
                oid_to_cid[taker_oid] = awaiting.popleft()
            if len(fill) >= 11:
                server_legs_by_oid[taker_oid] = [
                    int(value) for value in fill[6:11]
                ]
            return

        if "E" in msg:
            if not awaiting:
                _record_error(self.metrics, "unbound_error")
                return
            cid = awaiting.popleft()
            if pending_by_cid.pop(cid, None) is None:
                _record_error(self.metrics, "stale_error")
                return
            reason = self._reject_reason(msg) or "protocol_error"
            counts = Counter(self.metrics.rejected_by_reason)
            counts[reason] += 1
            self.metrics.rejected_by_reason = dict(counts)
            self.metrics.rejected += 1
            self.metrics.completed += 1
            return

        if "U" not in msg:
            _record_error(self.metrics, "unknown_frame")
            return

        update = msg.get("U", [])
        if len(update) < 2:
            _record_error(self.metrics, "short_update")
            return
        oid = str(update[0])
        cid = oid_to_cid.get(oid)
        if cid is None and awaiting:
            cid = awaiting.popleft()
            oid_to_cid[oid] = cid
        if cid is None:
            # User streams may carry lifecycle updates for orders this
            # connection did not submit. They are outside this worker's
            # latency accounting boundary.
            return
        sent_ns = pending_by_cid.pop(cid, None)
        if sent_ns is None:
            # Accepted GTC orders can emit later fill/update frames after the
            # first response already closed their latency sample.
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
            stamps = server_legs_by_oid.pop(oid, None)
            update_stamps = None
            if len(update) >= 10:
                update_stamps = [int(value) for value in update[5:10]]
            if stamps is not None or update_stamps is not None:
                fill_stamps = stamps or [0, 0, 0, 0, 0]
                update_values = update_stamps or [0, 0, 0, 0, 0]
                (
                    fill_gw_in, fill_risk_in, fill_me_in,
                    fill_match_done, fill_gw_out,
                ) = fill_stamps
                (
                    update_gw_in, update_risk_in, update_me_in,
                    update_match_done, update_gw_out,
                ) = update_values
                gw_in = update_gw_in or fill_gw_in
                risk_in = update_risk_in or fill_risk_in
                me_in = update_me_in or fill_me_in
                match_done = update_match_done or fill_match_done
                gw_out = update_gw_out or fill_gw_out
                required = (gw_in, risk_in, match_done, gw_out)
                reason = None
                if not all(required):
                    reason = "missing_completion_timestamp"
                elif not gw_in <= risk_in <= match_done <= gw_out:
                    reason = "non_monotone_completion_timestamp"
                elif oid in self.filled_oids and not me_in:
                    reason = "missing_matching_timestamp"
                elif oid in self.filled_oids and not risk_in <= me_in <= match_done:
                    reason = "non_monotone_matching_timestamp"
                if reason:
                    counts = Counter(self.metrics.timestamp_errors)
                    counts[reason] += 1
                    self.metrics.timestamp_errors = dict(counts)
                else:
                    legs = self.metrics.server_latencies_ns
                    legs["internal"].append(gw_out - gw_in)
                    legs["gateway_to_risk"].append(risk_in - gw_in)
                    legs["match_done_to_gateway"].append(
                        gw_out - match_done)
                if reason is None and oid in self.filled_oids:
                    legs["risk_to_me"].append(me_in - risk_in)
                    legs["matching"].append(
                        match_done - me_in)
            else:
                counts = Counter(self.metrics.timestamp_errors)
                counts["missing_completion_timestamp"] += 1
                self.metrics.timestamp_errors = dict(counts)
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

    async def run_worker(
        self,
        rate_per_worker: float,
        duration: int,
        start_barrier: asyncio.Barrier,
    ):
        """Send open-loop at the configured rate and drain responses."""
        interval = 1.0 / rate_per_worker if rate_per_worker > 0 else 0.1

        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(
                    self.config.gateway_url,
                    headers=self._headers(),
                    timeout=aiohttp.ClientTimeout(total=5),
                ) as ws:
                    await asyncio.wait_for(start_barrier.wait(), timeout=10)
                    if self.config.start_at_ns:
                        delay = (self.config.start_at_ns - time.time_ns()) / 1e9
                        if delay > 0:
                            await asyncio.sleep(delay)
                    loop = asyncio.get_running_loop()
                    awaiting = deque()
                    pending_by_cid: dict[str, int] = {}
                    oid_to_cid: dict[str, str] = {}
                    server_legs_by_oid: dict[str, list[int]] = {}
                    send_done = asyncio.Event()

                    async def sender():
                        started = loop.time()
                        next_send = started + self.pacing_index / self.config.rate
                        while loop.time() - started < duration:
                            if next_send - started >= duration:
                                break
                            delay = next_send - loop.time()
                            if delay > 0:
                                await asyncio.sleep(delay)
                            self.metrics.offered += 1
                            cid = None
                            try:
                                frame = self.generate_order()
                                cid = self._client_order_id(frame)
                                pending_by_cid[cid] = time.perf_counter_ns()
                                awaiting.append(cid)
                                await ws.send_str(frame)
                            except Exception:
                                if cid is not None:
                                    pending_by_cid.pop(cid, None)
                                    try:
                                        awaiting.remove(cid)
                                    except ValueError:
                                        pass
                                self.metrics.send_errors += 1
                                next_send += interval
                                continue
                            self.metrics.submitted += 1
                            self.metrics.fill_attempts += 1
                            next_send += interval
                        send_done.set()

                    async def heartbeater():
                        while not send_done.is_set():
                            try:
                                await asyncio.wait_for(
                                    send_done.wait(), timeout=4.0)
                            except asyncio.TimeoutError:
                                await ws.send_str(json.dumps({
                                    "H": [int(time.time() * 1000)],
                                }))

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
                            if response.type in (
                                aiohttp.WSMsgType.CLOSE,
                                aiohttp.WSMsgType.CLOSED,
                                aiohttp.WSMsgType.CLOSING,
                            ):
                                return
                            if response.type != aiohttp.WSMsgType.TEXT:
                                _record_error(
                                    self.metrics,
                                    f"ws_{response.type.name.lower()}",
                                )
                                continue
                            try:
                                msg = json.loads(response.data)
                            except (TypeError, json.JSONDecodeError):
                                _record_error(self.metrics, "invalid_json")
                                continue
                            self._handle_response(
                                msg, awaiting, pending_by_cid, oid_to_cid,
                                server_legs_by_oid)

                    run_started = loop.time()
                    await asyncio.gather(sender(), receiver(), heartbeater())
                    self.metrics.elapsed_sec = loop.time() - run_started
                    self.metrics.timed_out += len(pending_by_cid)
                    pending_by_cid.clear()

        except (
            aiohttp.ClientConnectorError,
            aiohttp.ServerDisconnectedError,
            aiohttp.ClientConnectionError,
            OSError,
        ) as e:
            _record_error(self.metrics, "worker_exception")
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
    err = None
    if config.probe_gateway:
        err = await _probe_gateway(
            config.gateway_url, config.jwt_secret
        )
    if err:
        return {
            "error": err,
            "config": {
                "target_rate": config.rate,
                "duration": config.duration,
                "users": config.users,
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
    start_barrier = asyncio.Barrier(config.connections)
    tasks = [
        worker.run_worker(
            rate_per_worker,
            config.duration,
            start_barrier,
        )
        for worker in workers
    ]

    if not config.quiet:
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
                "users": config.users,
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
    all_server_latencies = {
        "internal": [],
        "gateway_to_risk": [],
        "risk_to_me": [],
        "matching": [],
        "match_done_to_gateway": [],
    }

    for worker in workers:
        total_metrics.offered += worker.metrics.offered
        total_metrics.submitted += worker.metrics.submitted
        total_metrics.accepted += worker.metrics.accepted
        total_metrics.rejected += worker.metrics.rejected
        total_metrics.completed += worker.metrics.completed
        total_metrics.timed_out += worker.metrics.timed_out
        total_metrics.pending += worker.metrics.pending
        total_metrics.errors += worker.metrics.errors
        counts = Counter(total_metrics.errors_by_reason)
        counts.update(worker.metrics.errors_by_reason)
        total_metrics.errors_by_reason = dict(counts)
        total_metrics.send_errors += worker.metrics.send_errors
        total_metrics.fill_attempts += worker.metrics.fill_attempts
        total_metrics.filled_orders += worker.metrics.filled_orders
        total_metrics.fills += worker.metrics.fills
        counts = Counter(total_metrics.timestamp_errors)
        counts.update(worker.metrics.timestamp_errors)
        total_metrics.timestamp_errors = dict(counts)
        counts = Counter(total_metrics.rejected_by_reason)
        counts.update(worker.metrics.rejected_by_reason)
        total_metrics.rejected_by_reason = dict(counts)
        all_latencies.extend(worker.metrics.latencies_us)
        for leg, samples in worker.metrics.server_latencies_ns.items():
            all_server_latencies[leg].extend(samples)

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

    def latency_summary(data):
        data.sort()
        return {
            "samples": len(data),
            "p50": round(percentile(data, 50)) if data else None,
            "p95": round(percentile(data, 95)) if data else None,
            "p99": round(percentile(data, 99)) if data else None,
            "max": data[-1] if data else None,
        }

    run_elapsed = max(
        (worker.metrics.elapsed_sec for worker in workers),
        default=elapsed,
    )
    if run_elapsed <= 0:
        run_elapsed = elapsed
    actual_rate = total_metrics.submitted / run_elapsed

    results = {
        "config": {
            "target_rate": config.rate,
            "duration": config.duration,
            "users": config.users,
            "connections": config.connections,
            "symbols": config.symbols,
            "bid_price": config.bid_price,
            "ask_price": config.ask_price,
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
            "errors_by_reason": total_metrics.errors_by_reason,
            "send_errors": total_metrics.send_errors,
            "fill_attempts": total_metrics.fill_attempts,
            "filled_orders": total_metrics.filled_orders,
            "fills": total_metrics.fills,
            "timestamp_errors": total_metrics.timestamp_errors,
            "elapsed_sec": round(run_elapsed, 2),
            "actual_rate": round(actual_rate, 2),
            "achieved_rate": round(
                total_metrics.completed / run_elapsed, 2),
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
        },
        "server_latency_ns": {
            leg: latency_summary(samples)
            for leg, samples in all_server_latencies.items()
        },
        "_latency_samples_us": all_latencies,
        "_server_latency_samples_ns": all_server_latencies,
    }

    per_symbol = {}
    for symbol_id in config.symbols:
        symbol_workers = [
            worker for worker in workers if worker.symbol_id == symbol_id
        ]
        symbol_latencies = [
            sample for worker in symbol_workers
            for sample in worker.metrics.latencies_us
        ]
        symbol_legs = {
            leg: [
                sample for worker in symbol_workers
                for sample in worker.metrics.server_latencies_ns[leg]
            ]
            for leg in all_server_latencies
        }
        per_symbol[str(symbol_id)] = {
            "metrics": {
                key: sum(getattr(worker.metrics, key)
                         for worker in symbol_workers)
                for key in (
                    "offered", "submitted", "accepted", "rejected",
                    "completed", "timed_out", "pending", "errors",
                    "send_errors", "fill_attempts", "filled_orders", "fills",
                )
            },
            "latency_us": latency_summary(symbol_latencies),
            "server_latency_ns": {
                leg: latency_summary(samples)
                for leg, samples in symbol_legs.items()
            },
            "_latency_samples_us": symbol_latencies,
            "_server_latency_samples_ns": symbol_legs,
        }
    results["per_symbol"] = per_symbol

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
