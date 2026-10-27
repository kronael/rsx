# pyright: basic
"""The order routes drive the real rsx-cli binary against a fake gateway.

Run with: cd rsx-playground && uv run pytest tests/api_order_glue_test.py -v
"""

import pytest
from fake_gateway import SYMBOLS
from fake_gateway import FakeGateway
from fake_gateway import fill
from fake_gateway import resting
from fake_gateway import update
from fastapi.testclient import TestClient

import order_cli
import rsx_cli
import server

MID = 49000
FORM = {"symbol_id": "10", "side": "buy", "price": "0.049", "qty": "30"}
ADMIN_ROUTES = [
    "/api/orders/test",
    "/api/orders/quick",
    "/api/orders/batch",
    "/api/orders/random",
    "/api/orders/invalid",
    "/api/orders/x/cancel",
    "/api/gateway/duplicate-demo",
]


@pytest.fixture
def client():
    return TestClient(server.app)


@pytest.fixture(autouse=True)
def fresh_orders(monkeypatch):
    server.recent_orders.clear()
    monkeypatch.delenv("DATABASE_URL", raising=False)
    monkeypatch.setattr(order_cli, "ORDER_TIMEOUT_MS", 600)
    yield
    server.recent_orders.clear()


@pytest.fixture
def start_gateway(monkeypatch):
    started = []

    def start(answer=resting, symbols=SYMBOLS):
        fake = FakeGateway(answer, symbols)
        started.append(fake)
        monkeypatch.setattr(server, "ORDER_GATEWAY", order_cli.Gateway(fake.url))
        return fake

    yield start
    for fake in started:
        fake.stop()


@pytest.fixture
def book(monkeypatch):
    monkeypatch.setattr(
        server, "_book_mid", lambda symbol_id: MID if symbol_id == 10 else None
    )


def last_order():
    return server.recent_orders[-1]


def test_a_resting_order_reaches_the_gateway_and_cancels_through_it(
    client, start_gateway
):
    def answer(index, frame):
        return resting(index, frame) if "N" in frame else [update(2, 0, 300000)]

    fake = start_gateway(answer)

    placed = client.post("/api/orders/test", data=FORM)
    order = last_order()
    cid = order["cid"]
    assert placed.status_code == 200
    assert f"order {cid} resting" in placed.text
    assert order["status"] == "resting"
    assert order["user_id"] == 1
    assert fake.frames[0] == {"N": [10, 0, MID, 300000, cid, 0, 0, 0]}
    assert fake.authorizations[0].startswith("Bearer ")

    cancelled = client.post(f"/api/orders/{cid}/cancel")
    assert f"{cid} cancelled" in cancelled.text
    assert fake.frames[1] == {"C": [cid]}
    assert last_order()["status"] == "cancelled"


def test_the_trace_follows_a_placed_order_by_its_cid(client, start_gateway):
    start_gateway()
    client.post("/api/orders/test", data=FORM)
    cid = last_order()["cid"]

    trace = client.get("/x/order-trace", params={"trace-oid": cid})
    assert cid in trace.text
    assert "limit rests in the book" in trace.text


def test_human_units_use_the_decimals_the_gateway_serves(client, start_gateway):
    served = {"symbols": [{**SYMBOLS["symbols"][0], "price_decimals": 8,
                           "qty_decimals": 8}]}
    fake = start_gateway(symbols=served)

    client.post("/api/orders/test", data=FORM)
    assert fake.frames[0]["N"][2:4] == [4900000, 3000000000]


def test_a_price_with_more_decimals_than_the_symbol_is_refused(
    client, start_gateway
):
    fake = start_gateway()

    answer = client.post("/api/orders/test", data={**FORM, "price": "0.0490001"})
    assert "0.0490001 has more than 6 decimals" in answer.text
    assert fake.frames == []
    assert server.recent_orders == []


def test_a_fill_is_recorded_with_its_round_trip(client, start_gateway):
    def answer(index, frame):
        return [fill(MID, 300000), update(0, 300000, 0)]

    start_gateway(answer)
    form = {**FORM, "tif": "IOC"}

    done = client.post("/api/orders/test", data=form)
    assert f"order {last_order()['cid']} filled (" in done.text
    assert last_order()["status"] == "filled"
    assert last_order()["latency_us"] > 0


def test_a_reject_surfaces_the_failure_reason(client, start_gateway):
    start_gateway(lambda index, frame: [update(3, reason=14)])

    rejected = client.post("/api/orders/test", data=FORM)
    cid = last_order()["cid"]
    assert rejected.status_code == 200
    assert f"order {cid} rejected: ORDER_TOO_SMALL" in rejected.text
    assert last_order()["status"] == "rejected"
    assert last_order()["reason"] == "ORDER_TOO_SMALL"


def test_a_gateway_error_frame_surfaces_its_message(client, start_gateway):
    start_gateway(lambda index, frame: [{"E": [1009, "lot size misaligned"]}])

    rejected = client.post("/api/orders/test", data=FORM)
    assert "rejected: lot size misaligned" in rejected.text
    assert last_order()["status"] == "rejected"


def test_a_market_order_sweeps_the_book_as_ioc(client, start_gateway, book):
    fake = start_gateway(lambda index, frame: [update(2)])
    form = {**FORM, "price": "0", "tif": "GTC"}

    client.post("/api/orders/test", data=form)
    assert fake.frames[0]["N"][2:4] == [MID * 3 // 2, 300000]
    assert fake.frames[0]["N"][5] == 1
    assert last_order()["price"] == "mkt"


@pytest.mark.parametrize("qty", ["0", "-1", "abc"])
def test_a_non_positive_qty_is_not_sent(client, start_gateway, qty):
    fake = start_gateway()

    answer = client.post("/api/orders/test", data={**FORM, "qty": qty})
    assert "qty" in answer.text
    assert fake.frames == []
    assert server.recent_orders == []


def test_a_qty_with_more_decimals_than_the_symbol_is_refused(client, start_gateway):
    fake = start_gateway()

    answer = client.post("/api/orders/test", data={**FORM, "qty": "0.00001"})
    assert "0.00001 has more than 4 decimals" in answer.text
    assert fake.frames == []
    assert server.recent_orders == []


def test_a_down_gateway_is_an_error_row(client, monkeypatch):
    monkeypatch.setattr(server, "ORDER_GATEWAY", order_cli.Gateway("ws://127.0.0.1:1"))

    answer = client.post("/api/orders/test", data=FORM)
    assert "order error:" in answer.text
    assert last_order()["status"] == "error"
    assert last_order()["error"]


def test_a_missing_binary_is_reported(client, monkeypatch, tmp_path):
    monkeypatch.setattr(rsx_cli, "BINARY", tmp_path / "rsx-cli")

    answer = client.post("/api/orders/test", data=FORM)
    assert "cannot start" in answer.text
    assert last_order()["status"] == "error"


@pytest.mark.allow_5xx
def test_no_answer_is_a_timeout_that_may_still_rest(client, start_gateway):
    start_gateway(lambda index, frame: [])

    answer = client.post("/api/orders/test", data=FORM)
    assert answer.status_code == 504
    assert "the order may still rest" in answer.text
    assert last_order()["status"] == "timeout"


def test_a_rejected_cancel_says_why(client, start_gateway):
    def answer(index, frame):
        return resting(index, frame) if "N" in frame else [{"E": [1005, "not found"]}]

    start_gateway(answer)
    client.post("/api/orders/test", data=FORM)

    refused = client.post(f"/api/orders/{last_order()['cid']}/cancel")
    assert "not cancelled (rejected: not found)" in refused.text
    assert last_order()["status"] == "resting"


def test_batch_places_real_passive_orders_on_both_sides(
    client, start_gateway, book
):
    fake = start_gateway()

    answer = client.post("/api/orders/batch?count=4")
    assert "4 batch orders submitted: 4 resting" in answer.text
    assert len(fake.frames) == 4
    sides = [frame["N"][1] for frame in fake.frames]
    prices = [frame["N"][2] for frame in fake.frames]
    assert sides == [0, 1, 0, 1]
    assert all(px < MID for px, side in zip(prices, sides) if side == 0)
    assert all(px > MID for px, side in zip(prices, sides) if side == 1)
    for frame in fake.frames:
        _, _, px, qty, _, tif, _, _ = frame["N"]
        assert qty % 100000 == 0
        assert px * qty >= 10**10
        assert tif == 0
    cids = [frame["N"][4] for frame in fake.frames]
    assert [order["cid"] for order in server.recent_orders] == cids
    assert {order["status"] for order in server.recent_orders} == {"resting"}


def test_batch_rejects_are_counted_with_their_reasons(client, start_gateway, book):
    def answer(index, frame):
        return [update(3, reason=9)] if index % 2 else resting(index, frame)

    start_gateway(answer)

    result = client.post("/api/orders/batch?count=4")
    assert "2 resting, 2 rejected (RATE_LIMIT)" in result.text
    statuses = [order["status"] for order in server.recent_orders]
    assert statuses == ["resting", "rejected", "resting", "rejected"]


def test_batch_without_a_book_sends_nothing(client, start_gateway, monkeypatch):
    fake = start_gateway()
    monkeypatch.setattr(server, "_book_mid", lambda symbol_id: None)

    answer = client.post("/api/orders/batch")
    assert "no book to price against" in answer.text
    assert fake.frames == []


def test_batch_with_a_down_gateway_reports_it(client, book, monkeypatch):
    monkeypatch.setattr(server, "ORDER_GATEWAY", order_cli.Gateway("ws://127.0.0.1:1"))

    answer = client.post("/api/orders/batch")
    assert "batch orders error:" in answer.text
    assert server.recent_orders == []


def test_random_orders_go_to_symbols_with_a_book(client, start_gateway, book):
    fake = start_gateway()

    answer = client.post("/api/orders/random")
    assert "5 random orders submitted: 5 resting" in answer.text
    assert [frame["N"][0] for frame in fake.frames] == [10] * 5
    assert len(server.recent_orders) == 5


def test_random_without_a_running_book_sends_nothing(client, start_gateway, monkeypatch):
    fake = start_gateway()
    monkeypatch.setattr(server, "_book_mid", lambda symbol_id: None)

    answer = client.post("/api/orders/random")
    assert "random orders not sent" in answer.text
    assert fake.frames == []


def test_invalid_sends_a_misaligned_order_the_gateway_rejects(client, start_gateway):
    fake = start_gateway(lambda index, frame: [update(3, reason=1)])

    answer = client.post("/api/orders/invalid")
    assert "invalid order rejected: INVALID_LOT_SIZE" in answer.text
    assert fake.frames[0]["N"][3] == 1
    assert last_order()["status"] == "rejected"


def test_quick_market_order_is_an_ioc_sweep_as_the_given_user(
    client, start_gateway, book
):
    fake = start_gateway(lambda index, frame: [fill(MID, 300000), update(0)])

    answer = client.post(
        "/api/orders/quick", data={"side": "sell", "qty": "10", "user_id": "7"}
    )
    assert "SELL 10 filled" in answer.text
    assert fake.frames[0]["N"][1:4:2] == [1, 100000]
    assert fake.frames[0]["N"][2] == MID // 2
    assert fake.frames[0]["N"][5] == 1
    assert last_order()["user_id"] == 7


def test_quick_limit_offset_rests_as_gtc(client, start_gateway, book):
    fake = start_gateway()

    client.post("/api/orders/quick", data={"qty": "10", "price_offset_pct": "-1"})
    assert fake.frames[0]["N"][2] == round(MID * 0.99)
    assert fake.frames[0]["N"][5] == 0
    assert last_order()["status"] == "resting"


def test_quick_with_a_down_gateway_reports_the_error(client, monkeypatch):
    monkeypatch.setattr(server, "ORDER_GATEWAY", order_cli.Gateway("ws://127.0.0.1:1"))

    answer = client.post("/api/orders/quick", data={"qty": "10"})
    assert "error:" in answer.text
    assert last_order()["status"] == "error"


@pytest.mark.parametrize("path", ADMIN_ROUTES)
def test_orders_need_a_loopback_caller_or_the_admin_token(
    path, start_gateway, book
):
    fake = start_gateway()
    outsider = TestClient(server.app, client=("203.0.113.9", 5000))

    refused = outsider.post(path, data=FORM)
    assert refused.status_code == 403
    assert fake.frames == []
    assert server.recent_orders == []


def test_the_duplicate_demo_sends_one_cid_twice_and_shows_the_duplicate_reason(
    client, start_gateway
):
    def answer(index, frame):
        return [update(2)] if index == 0 else [update(3, reason=3)]

    fake = start_gateway(answer)

    result = client.post("/api/gateway/duplicate-demo", data={"user_id": "1"})
    assert result.status_code == 200
    assert "answered: duplicate order id" in result.text
    assert "rejected: DUPLICATE_ORDER_ID" in result.text
    first, second = (frame["N"][4] for frame in fake.frames)
    assert first == second


@pytest.mark.allow_5xx
def test_the_duplicate_demo_fails_when_the_repeat_is_not_a_duplicate(
    client, start_gateway
):
    start_gateway(lambda index, frame: [update(2)])

    result = client.post("/api/gateway/duplicate-demo", data={"user_id": "1"})
    assert result.status_code == 502
    assert "was not answered as a duplicate order id" in result.text
