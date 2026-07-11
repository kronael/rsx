"""Shared RSX local runtime planning.

The default profile is laptop-safe: no CPU affinity env vars are emitted.
Explicit pinning is for lab/perf runs where the operator controls isolated
cores and wants the hot-path layout.
"""

from __future__ import annotations

import os
import sys

SYMBOLS = {
    "PENGU": {
        "id": 10, "tick": 1, "lot": 100000,
        "price_dec": 6, "qty_dec": 4, "cap": "small",
    },
    "WIF": {
        "id": 11, "tick": 1, "lot": 100000,
        "price_dec": 6, "qty_dec": 4, "cap": "small",
    },
    "BONK": {
        "id": 12, "tick": 1, "lot": 100000,
        "price_dec": 6, "qty_dec": 4, "cap": "small",
    },
    "PEPE": {
        "id": 13, "tick": 1, "lot": 100000,
        "price_dec": 6, "qty_dec": 4, "cap": "small",
    },
    "DOGE": {
        "id": 14, "tick": 1, "lot": 100000,
        "price_dec": 6, "qty_dec": 4, "cap": "small",
    },
    "SOL": {
        "id": 3, "tick": 1, "lot": 10000,
        "price_dec": 4, "qty_dec": 6, "cap": "large",
    },
    "BTC": {
        "id": 1, "tick": 50, "lot": 100,
        "price_dec": 2, "qty_dec": 8, "cap": "large",
    },
    "ETH": {
        "id": 2, "tick": 10, "lot": 1000,
        "price_dec": 2, "qty_dec": 8, "cap": "large",
    },
}

SYMBOL_ORDER = [
    "PENGU", "SOL", "BTC", "ETH",
    "WIF", "BONK", "PEPE", "DOGE",
]

SCENARIOS = {
    "minimal": {"symbols": 1, "replication": "none", "gateways": 1},
    "1Z": {"symbols": 1, "replication": "none", "gateways": 1},
    "standard": {"symbols": 1, "replication": "replica", "gateways": 1},
    "1": {"symbols": 1, "replication": "replica", "gateways": 1},
    "duo": {"symbols": 2, "replication": "none", "gateways": 1},
    "2Z": {"symbols": 2, "replication": "none", "gateways": 1},
    "full": {"symbols": 3, "replication": "replica", "gateways": 1},
    "3": {"symbols": 3, "replication": "replica", "gateways": 1},
    "trio": {"symbols": 3, "replication": "none", "gateways": 1},
    "3Z": {"symbols": 3, "replication": "none", "gateways": 1},
    "stress": {"symbols": 3, "replication": "spare", "gateways": 2},
    "M3S": {"symbols": 3, "replication": "spare", "gateways": 2},
    "stress-low": {
        "symbols": 3,
        "replication": "none",
        "gateways": 1,
        "load": {"rate": 10, "duration": 60, "type": "sustained"},
    },
    "stress-high": {
        "symbols": 3,
        "replication": "replica",
        "gateways": 1,
        "load": {"rate": 100, "duration": 60, "type": "sustained"},
    },
    "stress-ultra": {
        "symbols": 4,
        "replication": "replica",
        "gateways": 2,
        "load": {"rate": 500, "duration": 10, "type": "burst"},
    },
}

UI_SCENARIOS = [
    "minimal",
    "duo",
    "full",
    "stress-low",
    "stress-high",
    "stress-ultra",
]

BASE_ME_CAST = 9100
BASE_RISK_CAST = 9200
BASE_GW_CAST = 9300
BASE_MARK_CAST = 9400
BASE_MD_CAST = 9500
# Gateway WS listen base. NOT 8080: the arizuko_routd docker container
# publishes host :8080, so the gateway can't bind it ("Address already in
# use") and every order WS 404s. 8088 is dedicated to the RSX gateway.
BASE_GW_WS = 8088
BASE_MD_WS = 8180
BASE_RISK_MARK_CAST = 9600
BASE_ME_REPLICATION = 9700

BASE_ME_HEALTH = 9800
BASE_RISK_HEALTH = 9900
BASE_GW_HEALTH = 10000
BASE_MARK_HEALTH = 10100
BASE_MD_HEALTH = 10200
BASE_RECORDER_HEALTH = 10300

DEFAULT_PG_URL = "postgres://rsx:rsx@127.0.0.1:5432/rsx"
PG_CONTAINER = "rsx-postgres"

# Optional perf profile layout. Demo/default does not emit these env vars.
CORE_GW = 1
CORE_RISK = 2
CORE_ME_0 = 3
CORE_MD = 4


def select_symbols(count, override_list=None):
    """Pick symbols by count from SYMBOL_ORDER."""
    if override_list:
        names = [s.strip().upper() for s in override_list.split(",")]
        return [SYMBOLS[n] | {"name": n} for n in names if n in SYMBOLS]
    return [SYMBOLS[n] | {"name": n} for n in SYMBOL_ORDER[:count]]


def resolve_scenario(args):
    """Resolve CLI args into spawn config dict (CLI overrides win)."""
    scenario = args.scenario or "minimal"
    preset = SCENARIOS.get(scenario)
    if not preset:
        print(f"unknown scenario: {scenario}")
        print(f"known: {', '.join(SCENARIOS)}")
        sys.exit(1)

    symbols = select_symbols(preset["symbols"], args.symbols)
    gateways = args.gateways if args.gateways is not None else preset["gateways"]
    replication = (
        args.replication
        if args.replication is not None
        else preset["replication"]
    )
    risk_shards = args.risk_shards if args.risk_shards is not None else 1

    return {
        "symbols": symbols,
        "gateways": int(gateways),
        "risk_shards": int(risk_shards),
        "replication": replication,
    }


def _with_core(env, key, core, pin_cores):
    if pin_cores:
        env[key] = str(core)
    return env


def _warn_if_pin_plan_exceeds_host(plan):
    requested = []
    for _name, _binary, env in plan:
        for key, value in env.items():
            if key.endswith("_CORE_ID"):
                requested.append(int(value))
    if not requested:
        return
    cpu_count = os.cpu_count() or 0
    if cpu_count and max(requested) >= cpu_count:
        print(
            "warning: pinned runtime requests core "
            f"{max(requested)} but host reports {cpu_count} CPUs",
            file=sys.stderr,
        )


def build_spawn_plan(config, pg_url, release=False, pin_cores=False):
    """Build ordered list of (name, binary, env) tuples."""
    plan = []
    symbols = config["symbols"]
    replication = config["replication"]
    target = "./target/release" if release else "./target/debug"

    mark_pairs = []
    for s in symbols:
        sid = s["id"]
        name = s["name"]
        mark_pairs.append(f"{name}={sid}")
        mark_pairs.append(f"{name}USDT={sid}")
        mark_pairs.append(f"{name}-USD={sid}")
    mark_symbol_map = ",".join(mark_pairs)

    streams = "/".join(f"{s['name'].lower()}usdt@trade" for s in symbols)
    binance_ws_url = (
        f"wss://stream.binance.com:9443/stream?streams={streams}"
    )

    # Risk consumes ONE ME replication stream for warm-catchup (it binds a
    # single ReplicationConsumer against RSX_ME_REPLICATION_ADDR using the
    # first ME's symbol_id as the stream_id — see rsx-risk main.rs). So this
    # env var must be a SINGLE addr, not a comma list: a joined blob is
    # parsed as one hostname and fails DNS ("Name or service not known"),
    # wedging risk in WarmCatchup forever so it never goes Live. Point it at
    # the first symbol's replay server (matches marketdata's RSX_MD_REPLAY_ADDR).
    me_replication_addr = f"127.0.0.1:{BASE_ME_REPLICATION + symbols[0]['id']}"
    me_cast_addrs = ",".join(
        f"127.0.0.1:{BASE_ME_CAST + s['id']}" for s in symbols
    )

    for sym in symbols:
        sid = sym["id"]
        env = {
            "RSX_ME_SYMBOL_ID": str(sid),
            "RSX_ME_PRICE_DECIMALS": str(sym["price_dec"]),
            "RSX_ME_QTY_DECIMALS": str(sym["qty_dec"]),
            "RSX_ME_TICK_SIZE": str(sym["tick"]),
            "RSX_ME_LOT_SIZE": str(sym["lot"]),
            "RSX_ME_WAL_DIR": f"./tmp/wal/{sym['name'].lower()}",
            "RSX_ME_CAST_ADDR": f"127.0.0.1:{BASE_ME_CAST + sid}",
            "RSX_RISK_CAST_ADDR": f"127.0.0.1:{BASE_RISK_CAST}",
            "RSX_MD_CAST_ADDR": f"127.0.0.1:{BASE_MD_CAST + sid}",
            "RSX_ME_REPLICATION_BIND_ADDR":
                f"127.0.0.1:{BASE_ME_REPLICATION + sid}",
            "RSX_ME_DATABASE_URL": pg_url,
            "RSX_ME_HEALTH_ADDR": f"127.0.0.1:{BASE_ME_HEALTH + sid}",
            "RUST_LOG": "info",
        }
        me_offset = len([s for s in symbols if s["id"] < sid])
        _with_core(env, "RSX_ME_CORE_ID", CORE_ME_0 + me_offset, pin_cores)
        plan.append((f"me-{sym['name'].lower()}", f"{target}/rsx-matching", env))

    plan.append((
        "mark",
        f"{target}/rsx-mark",
        {
            "RSX_MARK_LISTEN_ADDR": f"127.0.0.1:{BASE_MARK_CAST}",
            "RSX_MARK_WAL_DIR": "./tmp/wal/mark",
            "RSX_MARK_STREAM_ID": "100",
            "RSX_MARK_SYMBOL_MAP": mark_symbol_map,
            "RSX_RISK_MARK_CAST_ADDR": f"127.0.0.1:{BASE_RISK_MARK_CAST}",
            "RSX_MARK_SOURCE_BINANCE_ENABLED": "1",
            "RSX_MARK_SOURCE_BINANCE_WS_URL": binance_ws_url,
            "RSX_MARK_SOURCE_COINBASE_ENABLED": "0",
            "RSX_MARK_HEALTH_ADDR": f"127.0.0.1:{BASE_MARK_HEALTH}",
            "RUST_LOG": "info",
        },
    ))

    for shard in range(config["risk_shards"]):
        env = {
            "RSX_RISK_SHARD_ID": str(shard),
            "RSX_RISK_SHARD_COUNT": str(config["risk_shards"]),
            "RSX_RISK_MAX_SYMBOLS": str(max(s["id"] for s in symbols) + 1),
            "RSX_RISK_CAST_ADDR": f"127.0.0.1:{BASE_RISK_CAST + shard}",
            "RSX_GW_CAST_ADDR": f"127.0.0.1:{BASE_GW_CAST}",
            "RSX_ME_CAST_ADDRS": me_cast_addrs,
            "RSX_ME_REPLICATION_ADDR": me_replication_addr,
            "RSX_RISK_WAL_DIR": "./tmp/wal",
            "RSX_RISK_MARK_CAST_ADDR": f"127.0.0.1:{BASE_RISK_MARK_CAST}",
            "RSX_MARK_CAST_ADDR": f"127.0.0.1:{BASE_MARK_CAST}",
            "RSX_RISK_HEALTH_ADDR": f"127.0.0.1:{BASE_RISK_HEALTH + shard}",
            "DATABASE_URL": pg_url,
            "RUST_LOG": "info",
        }
        _with_core(env, "RSX_RISK_CORE_ID", CORE_RISK + shard, pin_cores)
        plan.append((f"risk-{shard}", f"{target}/rsx-risk", env))

        if replication in ("replica", "spare"):
            replica_count = 2 if replication == "spare" else 1
            for r in range(replica_count):
                plan.append((
                    f"risk-{shard}-replica-{r}",
                    f"{target}/rsx-risk",
                    {
                        "RSX_RISK_SHARD_ID": str(shard),
                        "RSX_RISK_SHARD_COUNT": str(config["risk_shards"]),
                        "RSX_RISK_MAX_SYMBOLS": str(len(symbols)),
                        "RSX_RISK_CAST_ADDR":
                            f"127.0.0.1:{BASE_RISK_CAST + 10 + shard * 2 + r}",
                        "RSX_GW_CAST_ADDR": f"127.0.0.1:{BASE_GW_CAST}",
                        "RSX_ME_CAST_ADDRS": me_cast_addrs,
                        "RSX_RISK_WAL_DIR": "./tmp/wal",
                        "RSX_RISK_HEALTH_ADDR":
                            f"127.0.0.1:{BASE_RISK_HEALTH + 10 + shard * 2 + r}",
                        "DATABASE_URL": pg_url,
                        "RUST_LOG": "info",
                    },
                ))

    for gw in range(config["gateways"]):
        sym_env = {}
        for sym in symbols:
            sid = sym["id"]
            sym_env[f"RSX_SYMBOL_{sid}_TICK_SIZE"] = str(sym["tick"])
            sym_env[f"RSX_SYMBOL_{sid}_LOT_SIZE"] = str(sym["lot"])
        env = {
            "RSX_GW_LISTEN": f"0.0.0.0:{BASE_GW_WS + gw}",
            "RSX_GW_CAST_ADDR": f"127.0.0.1:{BASE_GW_CAST + gw}",
            "RSX_RISK_CAST_ADDR": f"127.0.0.1:{BASE_RISK_CAST}",
            "RSX_GW_WAL_DIR": "./tmp/wal",
            "RSX_GW_JWT_SECRET": "rsx-dev-secret-not-for-prod-padpad",
            "RSX_GW_RL_USER": "5000",
            "RSX_GW_RL_IP": "10000",
            "RSX_MAX_SYMBOLS": "16",
            "RSX_GW_HEALTH_ADDR": f"127.0.0.1:{BASE_GW_HEALTH + gw}",
            **sym_env,
            "RUST_LOG": "info",
        }
        _with_core(env, "RSX_GW_CORE_ID", CORE_GW + gw, pin_cores)
        plan.append((f"gw-{gw}", f"{target}/rsx-gateway", env))

    first_sid = symbols[0]["id"]
    env = {
        "RSX_MD_LISTEN": f"0.0.0.0:{BASE_MD_WS}",
        "RSX_ME_CAST_ADDRS": me_cast_addrs,
        "RSX_MD_STREAM_ID": str(first_sid),
        "RSX_MD_REPLAY_ADDR": f"127.0.0.1:{BASE_ME_REPLICATION + first_sid}",
        "RSX_MD_HEALTH_ADDR": f"127.0.0.1:{BASE_MD_HEALTH}",
        "RUST_LOG": "info",
    }
    _with_core(env, "RSX_MD_CORE_ID", CORE_MD, pin_cores)
    plan.append(("marketdata", f"{target}/rsx-marketdata", env))

    first = symbols[0]
    sid = first["id"]
    plan.append((
        "recorder",
        f"{target}/rsx-recorder",
        {
            "RSX_RECORDER_STREAM_ID": str(sid),
            "RSX_RECORDER_PRODUCER_ADDR":
                f"127.0.0.1:{BASE_ME_REPLICATION + sid}",
            "RSX_RECORDER_ARCHIVE_DIR": "./tmp/wal/archive",
            "RSX_RECORDER_TIP_FILE": f"./tmp/recorder-tip-{sid}",
            "RSX_RECORDER_HEALTH_ADDR": f"127.0.0.1:{BASE_RECORDER_HEALTH}",
            "RUST_LOG": "info",
        },
    ))

    _warn_if_pin_plan_exceeds_host(plan)
    return plan
