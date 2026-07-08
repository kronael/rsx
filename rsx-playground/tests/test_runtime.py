"""Runtime spawn-plan tests."""

from __future__ import annotations

import runtime


def _minimal_config():
    return {
        "symbols": runtime.select_symbols(1),
        "gateways": 1,
        "risk_shards": 1,
        "replication": "none",
    }


def _core_keys(plan):
    keys = set()
    for _name, _binary, env in plan:
        keys.update(k for k in env if k.endswith("_CORE_ID"))
    return keys


def test_minimal_spawn_plan_is_laptop_safe_by_default():
    plan = runtime.build_spawn_plan(
        _minimal_config(),
        runtime.DEFAULT_PG_URL,
    )

    assert _core_keys(plan) == set()


def test_minimal_spawn_plan_can_pin_cores_for_perf_runs():
    plan = runtime.build_spawn_plan(
        _minimal_config(),
        runtime.DEFAULT_PG_URL,
        pin_cores=True,
    )

    assert _core_keys(plan) == {
        "RSX_ME_CORE_ID",
        "RSX_RISK_CORE_ID",
        "RSX_GW_CORE_ID",
        "RSX_MD_CORE_ID",
    }
