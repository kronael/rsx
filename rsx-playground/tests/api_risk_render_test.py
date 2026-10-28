"""Unit tests of the risk page renderers in pages.py.

Run with: cd rsx-playground && uv run pytest tests/api_risk_render_test.py -v
"""

from test_utils import account
from test_utils import position


def heatmap_user(*nets, symbol_id=10):
    return {"positions": [
        {"symbol_id": symbol_id, "net": net} for net in nets]}


def ladder_user(user_id, margin_ratio=2.0, symbol_id=10):
    return {
        "user_id": user_id, "margin_ratio": margin_ratio,
        "equity": 1_000_000_000, "mm_required": 500_000_000,
        "positions": [{"symbol_id": symbol_id, "net": 5}],
    }


def test_render_position_heatmap_empty_list():
    from pages import render_position_heatmap
    out = render_position_heatmap([])
    assert "no open positions" in out


def test_render_position_heatmap_with_positions():
    from pages import render_position_heatmap
    users = [
        heatmap_user(100, -50),
        heatmap_user(200, symbol_id=1),
    ]
    out = render_position_heatmap(users)
    assert "<table" in out
    assert "PENGU" in out
    assert "BTC" in out


def test_render_position_heatmap_flat_position():
    from pages import render_position_heatmap
    out = render_position_heatmap([heatmap_user(100, -100)])
    assert "<table" in out
    assert "text-slate-500" in out


def test_render_margin_ladder_empty():
    from pages import render_margin_ladder
    out = render_margin_ladder([])
    assert "no open positions" in out


def test_render_margin_ladder_flat_accounts_are_skipped():
    from pages import render_margin_ladder
    out = render_margin_ladder([{"user_id": 1, "positions": []}])
    assert "no open positions" in out


def test_render_margin_ladder_with_data():
    from pages import render_margin_ladder
    out = render_margin_ladder([ladder_user(7)])
    assert "<table" in out
    assert ">7<" in out


def test_render_margin_ladder_closest_to_liquidation_first():
    from pages import render_margin_ladder
    out = render_margin_ladder([
        ladder_user(1, margin_ratio=5.0),
        ladder_user(2, margin_ratio=1.1),
    ])
    assert out.index(">2<") < out.index(">1<")


def test_render_margin_ladder_capped_at_20():
    from pages import MARGIN_LADDER_MAX_ACCOUNTS
    from pages import render_margin_ladder
    users = [ladder_user(uid + 1000) for uid in range(30)]
    out = render_margin_ladder(users)
    assert out.count("<tr") - 1 == MARGIN_LADDER_MAX_ACCOUNTS


def test_render_funding_none():
    from pages import render_funding
    out = render_funding(None)
    assert "no BBO data available" in out


def test_render_funding_empty_dict():
    from pages import render_funding
    out = render_funding({})
    assert "no BBO data available" in out


def test_render_funding_with_data():
    from pages import render_funding
    stats = {
        10: {"bid_px": 390, "ask_px": 410,
             "bid_qty": 500, "ask_qty": 300},
    }
    out = render_funding(stats)
    assert "<table" in out
    assert "PENGU" in out


def test_render_funding_zero_bid_ask():
    from pages import render_funding
    stats = {
        1: {"bid_px": 0, "ask_px": 0,
            "bid_qty": 0, "ask_qty": 0},
    }
    out = render_funding(stats)
    assert "--" in out


def test_render_risk_latency_empty():
    from pages import render_risk_latency
    out = render_risk_latency([])
    assert "p50" in out
    assert "--" in out


def test_render_risk_latency_none():
    from pages import render_risk_latency
    out = render_risk_latency(None)
    assert "p50" in out


def test_render_risk_latency_with_data():
    from pages import render_risk_latency
    latencies = [50, 80, 120, 200, 500, 300, 90, 60, 40, 70]
    out = render_risk_latency(latencies)
    assert "p50" in out
    assert "p95" in out
    assert "p99" in out
    assert "us" in out


def test_render_risk_latency_fast_is_neutral():
    from pages import render_risk_latency
    out = render_risk_latency([10, 20, 30])
    assert "slate-300" in out
    assert "amber-400" not in out
    assert "red-400" not in out


def test_render_risk_latency_slow_is_amber():
    from pages import render_risk_latency
    out = render_risk_latency([20_000, 30_000, 40_000])
    assert "amber-400" in out
    assert "red-400" not in out


def test_render_risk_latency_pathological_is_red():
    from pages import render_risk_latency
    out = render_risk_latency([100_000, 200_000, 300_000])
    assert "red-400" in out


def test_render_reconciliation_has_three_checks():
    from pages import render_reconciliation
    out = render_reconciliation()
    assert out.count("SKIP") == 3


def test_render_reconciliation_check_names():
    from pages import render_reconciliation
    out = render_reconciliation()
    lower = out.lower()
    assert "frozen margin" in lower
    assert "marketdata snapshot vs wal bbo" in lower
    assert "mark-process index" in lower


def test_render_risk_user_flat_account():
    from pages import render_risk_user
    out = render_risk_user(account(3))
    assert "user 3" in out
    assert "flat" in out


def test_render_risk_user_with_positions():
    from pages import render_risk_user
    out = render_risk_user(account(1, positions=[position(10, 100)]))
    assert "<table" in out
    assert "PENGU" in out
