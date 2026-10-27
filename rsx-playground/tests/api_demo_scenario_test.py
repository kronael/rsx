# pyright: basic
"""The demo scenario places its orders through rsx-cli against a fake gateway.

Run with: cd rsx-playground && uv run pytest tests/api_demo_scenario_test.py -v
"""

import re

import pytest
from fake_gateway import FakeGateway
from fake_gateway import fill
from fake_gateway import update

import demo_scenario
import order_cli

CID_SHAPE = re.compile(r"^[0-9A-Za-z_-]{1,20}$")
LOT = 100_000

# 1 USD on a 6+4 decimals symbol (PENGU): 10^10 raw.
ONE_USD = 10_000_000_000


def fills_whole_order(index, frame):
    _, _, price, qty, *_ = frame["N"]
    return [fill(price, qty), update(0, qty, 0)]


@pytest.fixture
def start_gateway(monkeypatch):
    started = []

    def start(answer):
        fake = FakeGateway(answer)
        started.append(fake)
        monkeypatch.setattr(demo_scenario, "GATEWAY", order_cli.Gateway(fake.url))
        return fake

    yield start
    for fake in started:
        fake.stop()


async def test_minimum_size_orders_meet_the_minimum_with_unique_cids(start_gateway):
    fake = start_gateway(fills_whole_order)

    for count in range(20):
        report = await demo_scenario.ioc(
            3, "buy" if count % 2 else "sell", 30_000 + count)
        assert report.filled > 0

    assert len(fake.frames) == 20
    cids = set()
    for frame in fake.frames:
        _, _, price, qty, cid, tif, *_ = frame["N"]
        assert price * qty >= ONE_USD
        assert qty % LOT == 0
        assert tif == 1
        assert CID_SHAPE.match(cid)
        cids.add(cid)
    assert len(cids) == 20


async def test_an_explicit_quantity_is_sent_as_given(start_gateway):
    fake = start_gateway(fills_whole_order)

    report = await demo_scenario.ioc(1, "buy", 31_000, 25_000_000)

    assert fake.frames[0]["N"][2:4] == [31_000, 25_000_000]
    assert (report.filled, report.avg_px()) == (25_000_000, 31_000)


async def test_a_rejected_order_raises_with_the_reason(start_gateway):
    start_gateway(lambda index, frame: [update(3, reason=14)])

    with pytest.raises(RuntimeError, match="user 2 order rejected"):
        await demo_scenario.ioc(2, "sell", 30_000, 1_000_000)
