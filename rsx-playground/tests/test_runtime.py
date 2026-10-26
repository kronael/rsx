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


def test_trio_assigns_each_me_distinct_risk_and_replay_streams():
    config = _minimal_config()
    config["symbols"] = runtime.select_symbols(3)
    plan = runtime.build_spawn_plan(config, runtime.DEFAULT_PG_URL)
    processes = {name: env for name, _binary, env in plan}

    risk = processes["risk-0"]
    assert risk["RSX_RISK_ME_RECV_ADDRS"] == (
        "10=127.0.0.1:28310,3=127.0.0.1:28303,1=127.0.0.1:28301"
    )
    assert risk["RSX_ME_REPLICATION_ADDRS"] == (
        "10=127.0.0.1:9710,3=127.0.0.1:9703,1=127.0.0.1:9701"
    )
    assert processes["me-pengu"]["RSX_RISK_ME_RECV_ADDR"].endswith(":28310")
    assert processes["me-sol"]["RSX_RISK_ME_RECV_ADDR"].endswith(":28303")
    assert processes["me-btc"]["RSX_RISK_ME_RECV_ADDR"].endswith(":28301")


def test_every_me_gets_the_playground_book_capacity():
    config = _minimal_config()
    config["symbols"] = runtime.select_symbols(3)
    plan = runtime.build_spawn_plan(config, runtime.DEFAULT_PG_URL)
    me_envs = [env for name, _binary, env in plan if name.startswith("me-")]

    assert len(me_envs) == 3
    assert {env["RSX_ME_BOOK_CAPACITY"] for env in me_envs} == {
        str(runtime.PLAYGROUND_BOOK_CAPACITY)
    }
