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


def test_catalog_min_notional_prefers_the_row_then_one_usd():
    assert notional.catalog_min_notional(
        {"min_notional": 30_000_000_000, "price_decimals": 6,
         "qty_decimals": 4}) == 30_000_000_000
    assert notional.catalog_min_notional(
        {"price_decimals": 6, "qty_decimals": 4}) == ONE_USD
    assert notional.catalog_min_notional(
        {"price_decimals": 2, "qty_decimals": 8}) == ONE_USD
    assert notional.catalog_min_notional({"id": 1}) == 0


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
