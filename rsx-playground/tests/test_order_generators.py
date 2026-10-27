"""Order generators: cid scheme and minimum-notional sizing.

Every in-repo generator mints cids from `order_ids` (spec 61 client
rules: 1-20 ASCII chars, never padded, a random session part plus a base36
counter) and sizes orders with `notional` so risk's minimum-notional check
passes (ORDER_TOO_SMALL otherwise).
"""

import asyncio
import json
import os
import re
import sys
from pathlib import Path

import pytest
from aiohttp import web

sys.path.insert(0, str(Path(__file__).parent.parent))

import notional
import order_ids
from order_ids import CidMint

CID_SHAPE = re.compile(r"^[0-9A-Za-z_-]{1,20}$")

# 1 USD on a 6+4 decimals symbol (PENGU): 10^10 raw.
ONE_USD = 10_000_000_000


# ── cid scheme ──────────────────────────────────────────────────────


def test_mint_never_repeats_within_a_session():
    mint = CidMint()
    cids = [mint.next() for _ in range(100_000)]
    assert len(set(cids)) == len(cids)


def test_mint_counters_one_ten_hundred_stay_distinct():
    # "s0-1".ljust(20, "0") == "s0-10".ljust(20, "0"): a padded scheme
    # collapses these, and the gateway answers the repeat as a duplicate.
    mint = CidMint("abcd1234")
    cids = [mint.next() for _ in range(100)]
    assert cids[0] == "abcd12341"
    assert cids[9] == "abcd1234a"
    assert cids[35] == "abcd123410"
    assert len({cids[0], cids[35], cids[99]}) == 3


def test_mints_of_two_restarts_share_no_cid():
    first = {CidMint().next() for _ in range(5)}
    second = {CidMint().next() for _ in range(5)}
    assert not first & second
    # Same counter range from two sessions, many restarts.
    minted = set()
    for _ in range(200):
        mint = CidMint()
        cids = {mint.next() for _ in range(200)}
        assert not minted & cids
        minted |= cids


def test_mint_cid_shape_is_unpadded_session_plus_base36_counter():
    mint = CidMint()
    assert len(mint.session) >= 8
    assert CID_SHAPE.match(mint.session)
    for count in range(1, 5_000):
        cid = mint.next()
        assert CID_SHAPE.match(cid)
        assert cid == mint.session + order_ids.base36(count)
        assert order_ids.base36(count) == (
            order_ids.base36(count).lstrip("0") or "0")


def test_session_carries_at_least_forty_random_bits():
    # 36^8 combinations; 200 draws all differ.
    assert 36 ** order_ids.SESSION_CHARS >= 2 ** 40
    assert len({CidMint().session for _ in range(200)}) == 200


def test_process_cid_prefix_counts_toward_the_limit():
    cid = order_ids.next_cid("probe-")
    assert cid.startswith("probe-")
    assert CID_SHAPE.match(cid)


def test_forked_child_draws_its_own_session():
    parent_session = order_ids.process_mint.session
    read_end, write_end = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(read_end)
        os.write(write_end, order_ids.process_mint.session.encode())
        os._exit(0)
    os.close(write_end)
    child_session = os.read(read_end, 64).decode()
    os.close(read_end)
    os.waitpid(pid, 0)
    assert child_session != parent_session
    assert order_ids.process_mint.session == parent_session


# ── sizing helpers ──────────────────────────────────────────────────


@pytest.mark.parametrize("price,lot,floor", [
    (30_000, 100_000, ONE_USD),          # PENGU 0.03
    (6_000_000, 1_000, ONE_USD),         # BTC 60000.00, lot 1e-5
    (31, 100_000, ONE_USD),
    (1, 1, 7),
    (999_999_999, 7, 10**18),
])
def test_min_order_qty_is_the_smallest_lot_multiple_at_the_floor(
        price, lot, floor):
    qty = notional.min_order_qty(floor, price, lot)
    assert qty % lot == 0
    assert qty >= lot
    assert price * qty >= floor
    if qty > lot:
        assert price * (qty - lot) < floor


def test_min_order_qty_exactly_at_the_floor_is_not_raised():
    # price 5, lot 10: 200 -> notional 1000.
    assert notional.min_order_qty(1000, 5, 10) == 200
    assert notional.min_order_qty(1001, 5, 10) == 210


def test_min_order_qty_is_one_lot_when_one_lot_already_clears():
    assert notional.min_order_qty(100, 1_000_000, 50) == 50
    assert notional.min_order_qty(0, 10, 50) == 50


def test_size_qty_raises_and_never_lowers():
    assert notional.size_qty(100_000, 30_000, ONE_USD, 100_000) == 400_000
    assert notional.size_qty(
        9_000_000, 30_000, ONE_USD, 100_000) == 9_000_000
    # an unaligned quantity is rounded up to the lot
    assert notional.size_qty(150_000, 30_000, ONE_USD, 100_000) == 400_000
    assert notional.size_qty(1_050_000, 30_000, ONE_USD, 100_000) == (
        1_100_000)


def test_sizing_rejects_nonpositive_price_and_lot():
    with pytest.raises(ValueError):
        notional.min_order_qty(ONE_USD, 0, 100)
    with pytest.raises(ValueError):
        notional.size_qty(10, 5, ONE_USD, 0)


def test_crossing_order_lots_covers_the_lower_price():
    # buy at 31 needs 322_581.. -> lots at ask; sell at 29 needs more.
    lots = notional.crossing_order_lots(29, 31, 100_000, ONE_USD)
    assert 29 * lots * 100_000 >= ONE_USD
    assert 31 * lots * 100_000 >= ONE_USD


def test_catalog_min_notional_prefers_the_row_then_one_usd():
    assert notional.catalog_min_notional(
        {"min_notional": 30_000_000_000, "price_decimals": 6,
         "qty_decimals": 4}) == 30_000_000_000
    assert notional.catalog_min_notional(
        {"price_decimals": 6, "qty_decimals": 4}) == ONE_USD
    assert notional.catalog_min_notional(
        {"price_decimals": 2, "qty_decimals": 8}) == ONE_USD
    assert notional.catalog_min_notional({"id": 1}) == 0


# ── stress client ───────────────────────────────────────────────────


def _stress_config(**kwargs):
    from stress_client import StressConfig
    return StressConfig(**kwargs)


def test_stress_cids_do_not_collide_over_n_orders():
    from stress_client import StressClient
    worker = StressClient(0, _stress_config(symbols=[10]))
    cids = [json.loads(worker.generate_order())["N"][4]
            for _ in range(20_000)]
    assert len(set(cids)) == len(cids)
    assert all(CID_SHAPE.match(cid) for cid in cids)
    # the padded scheme produced one string for orders 1, 10 and 100
    assert len({cids[0], cids[9], cids[99]}) == 3


def test_stress_cids_distinct_across_workers_and_restarts():
    from stress_client import StressClient
    config = _stress_config(symbols=[10], users=1)
    seen = set()
    for _restart in range(2):
        workers = [StressClient(i, config) for i in range(4)]
        for worker in workers:
            for _ in range(1_000):
                cid = json.loads(worker.generate_order())["N"][4]
                assert cid not in seen
                seen.add(cid)
    assert len(seen) == 2 * 4 * 1_000


def test_stress_orders_are_sized_to_the_symbol_minimum():
    from stress_client import StressClient
    config = _stress_config(
        symbols=[10, 1],
        symbol_configs={
            10: {"tick_size": 1, "lot_size": 100_000, "bid_price": 29_000,
                 "ask_price": 31_000, "min_notional": ONE_USD},
            1: {"tick_size": 50, "lot_size": 100, "bid_price": 6_999_950,
                "ask_price": 7_000_000, "min_notional": ONE_USD},
        },
    )
    for worker_id in (0, 1):
        worker = StressClient(worker_id, config)
        for _ in range(4):
            symbol, side, price, qty = json.loads(
                worker.generate_order())["N"][:4]
            values = config.symbol_configs[symbol]
            assert qty % values["lot_size"] == 0
            assert price * qty >= values["min_notional"]
            assert price * (qty - values["lot_size"]) < (
                values["min_notional"]) or qty == values["lot_size"]


def test_stress_single_symbol_fallback_uses_config_minimum():
    from stress_client import StressClient
    worker = StressClient(0, _stress_config(
        symbols=[10], lot_size=100_000, bid_price=29_000,
        ask_price=31_000, min_notional=ONE_USD))
    qty = json.loads(worker.generate_order())["N"][3]
    assert 31_000 * qty >= ONE_USD and qty % 100_000 == 0


def test_stress_rejects_a_negative_minimum():
    with pytest.raises(ValueError):
        _stress_config(min_notional=-1)


def test_stress_partitions_carry_the_minimum():
    from stress import _partition_configs
    config = _stress_config(
        symbols=[10], connections=4, rate=4, min_notional=ONE_USD)
    parts = _partition_configs(config, 2, 0)
    assert {part.min_notional for part in parts} == {ONE_USD}


def test_resolve_minimums_fills_from_the_catalog():
    from stress import resolve_minimums
    configs = {
        10: {"lot_size": 1},
        3: {"lot_size": 1, "min_notional": 5},
    }
    resolved, fallback = resolve_minimums(
        [10, 3], configs, 0, {10: ONE_USD, 3: ONE_USD})
    assert resolved[10]["min_notional"] == ONE_USD
    assert resolved[3]["min_notional"] == 5      # a config value wins
    assert fallback == ONE_USD                    # first symbol
    assert resolve_minimums([10], {}, 77, {10: ONE_USD})[1] == 77
    assert resolve_minimums([10], {}, 0, {})[1] == 0


# ── playground market maker (Python) ────────────────────────────────


def test_market_maker_cids_unique_over_n_orders_and_restarts():
    from market_maker import DummyMarketMaker
    seen = set()
    for _restart in range(2):
        maker = DummyMarketMaker()
        for _ in range(10_000):
            cid = maker._next_cid()
            assert CID_SHAPE.match(cid)
            assert cid not in seen
            seen.add(cid)


def test_market_maker_sizes_quotes_to_the_minimum():
    from market_maker import DummyMarketMaker
    for price in (29_000, 30_000, 31_007):
        qty = DummyMarketMaker._sized(100_000, price, ONE_USD, 100_000)
        assert qty % 100_000 == 0
        assert price * qty >= ONE_USD
    # a quote already above the minimum is untouched
    assert DummyMarketMaker._sized(
        5_000_000, 30_000, ONE_USD, 100_000) == 5_000_000


@pytest.mark.asyncio
async def test_market_maker_reads_the_minimum_from_the_catalog(monkeypatch):
    from market_maker import DummyMarketMaker

    async def symbols(_request):
        return web.json_response({"symbols": [
            {"id": 1, "tick_size": 1, "lot_size": 1000,
             "price_decimals": 2, "qty_decimals": 8,
             "min_notional": 30_000_000_000},
            {"id": 10, "tick_size": 1, "lot_size": 100_000,
             "price_decimals": 6, "qty_decimals": 4},
        ]})

    app = web.Application()
    app.router.add_get("/v1/symbols", symbols)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]
    monkeypatch.setenv("RSX_SYMBOLS_URL", f"http://127.0.0.1:{port}/v1/symbols")
    try:
        maker = DummyMarketMaker()
        await maker._fetch_tick_sizes("ws://127.0.0.1:1")
    finally:
        await runner.cleanup()
    assert maker.min_notionals == {1: 30_000_000_000, 10: ONE_USD}
    assert maker.lot_sizes == {1: 1000, 10: 100_000}


# ── demo scenario ───────────────────────────────────────────────────


class _FakeSocket:
    def __init__(self, sent):
        self.sent = sent

    async def send_str(self, text):
        self.sent.append(json.loads(text))

    async def receive(self, timeout=None):
        raise asyncio.TimeoutError

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return False


class _FakeSession:
    def __init__(self):
        self.sent = []

    def ws_connect(self, *_args, **_kwargs):
        return _FakeSocket(self.sent)


@pytest.mark.asyncio
async def test_demo_scenario_orders_meet_the_minimum_with_unique_cids(
        monkeypatch):
    import demo_scenario
    monkeypatch.setattr(demo_scenario, "_floor", ONE_USD)
    monkeypatch.setattr(demo_scenario, "CIDS", CidMint())
    session = _FakeSession()
    for count in range(50):
        await demo_scenario.ioc(
            session, 3, count % 2, 30_000 + count, 2 * 100_000,
            drain_s=0.01)
    assert len(session.sent) == 50
    cids = set()
    for frame in session.sent:
        symbol, side, price, qty, cid, tif = frame["N"]
        assert price * qty >= ONE_USD
        assert qty % demo_scenario.LOT == 0
        assert CID_SHAPE.match(cid)
        cids.add(cid)
    assert len(cids) == 50
