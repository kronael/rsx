#!/usr/bin/env python3
"""RSX local dev runner.

Usage:
    python run.py [SCENARIO] [OPTIONS]

Scenarios:
    minimal   1Z   PENGU, no replicas (default)
    standard  1    PENGU with replicas
    duo       2Z   PENGU + SOL, no replicas
    full      3    PENGU + SOL + BTC with replicas
    stress    M3S  multi-gw, 3 symbols, spare replicas

Options:
    --postgres docker|external  Postgres mode (default: external)
    --pg-url URL                Postgres URL
    --build                     Run cargo build first
    --release                   Build in release mode
    --dry-run                   Print spawn plan, don't run
    --symbols SYM,SYM,...       Override symbol selection
    --gateways N                Number of gateways
    --risk-shards N             Number of risk shards
    --replication none|replica|spare  Replication mode
"""

import argparse
import json
import os
import signal
import subprocess
import sys
import time

# ── symbol catalog ──────────────────────────────────────

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

# selection order: smallcap first, then alternate large
SYMBOL_ORDER = [
    "PENGU", "SOL", "BTC", "ETH",
    "WIF", "BONK", "PEPE", "DOGE",
]

# ── scenario presets ────────────────────────────────────

SCENARIOS = {
    "minimal":  {"symbols": 1, "replication": "none",
                 "gateways": 1},
    "1Z":       {"symbols": 1, "replication": "none",
                 "gateways": 1},
    "standard": {"symbols": 1, "replication": "replica",
                 "gateways": 1},
    "1":        {"symbols": 1, "replication": "replica",
                 "gateways": 1},
    "duo":      {"symbols": 2, "replication": "none",
                 "gateways": 1},
    "2Z":       {"symbols": 2, "replication": "none",
                 "gateways": 1},
    "full":     {"symbols": 3, "replication": "replica",
                 "gateways": 1},
    "3":        {"symbols": 3, "replication": "replica",
                 "gateways": 1},
    "stress":   {"symbols": 3, "replication": "spare",
                 "gateways": 2},
    "M3S":      {"symbols": 3, "replication": "spare",
                 "gateways": 2},
}

# ── port allocation ─────────────────────────────────────

BASE_ME_CMP = 9100
BASE_RISK_CMP = 9200
BASE_GW_CMP = 9300
BASE_MARK_CMP = 9400
BASE_MD_CMP = 9500
BASE_GW_WS = 8080
BASE_MD_WS = 8180
BASE_RISK_MARK_CMP = 9600

DEFAULT_PG_URL = (
    "postgres://rsx:rsx@127.0.0.1:5432/rsx"
)
PG_CONTAINER = "rsx-postgres"

# ── helpers ─────────────────────────────────────────────


def select_symbols(count, override_list=None):
    """Pick symbols by count from SYMBOL_ORDER."""
    if override_list:
        names = [s.strip().upper()
                 for s in override_list.split(",")]
        return [SYMBOLS[n] | {"name": n} for n in names
                if n in SYMBOLS]
    return [SYMBOLS[n] | {"name": n}
            for n in SYMBOL_ORDER[:count]]


def resolve_scenario(args):
    """Resolve CLI args into spawn config dict (CLI overrides win)."""
    scenario = args.scenario or "minimal"
    preset = SCENARIOS.get(scenario)
    if not preset:
        print(f"unknown scenario: {scenario}")
        print(f"known: {', '.join(SCENARIOS)}")
        sys.exit(1)

    # CLI args override preset
    symbols = select_symbols(
        preset["symbols"],
        args.symbols,
    )
    gateways = (args.gateways if args.gateways is not None
                else preset["gateways"])
    replication = (args.replication if args.replication is not None
                   else preset["replication"])
    risk_shards = (args.risk_shards if args.risk_shards is not None
                   else 1)

    return {
        "symbols": symbols,
        "gateways": int(gateways),
        "risk_shards": int(risk_shards),
        "replication": replication,
    }


def build_spawn_plan(config, pg_url, release=False):
    """Build ordered list of (name, binary, env) tuples."""
    plan = []
    symbols = config["symbols"]
    replication = config["replication"]

    target = "./target/release" if release else "./target/debug"

    # symbol map for mark: "PENGU=10,SOL=3,..."
    mark_symbol_map = ",".join(
        f"{s['name']}={s['id']}" for s in symbols
    )

    # ── ME instances ────────────────────────────────
    for sym in symbols:
        sid = sym["id"]
        me_cmp = f"127.0.0.1:{BASE_ME_CMP + sid}"
        risk_cmp = f"127.0.0.1:{BASE_RISK_CMP}"
        md_cmp = f"127.0.0.1:{BASE_MD_CMP + sid}"
        env = {
            "RSX_ME_SYMBOL_ID": str(sid),
            "RSX_ME_PRICE_DECIMALS": str(sym["price_dec"]),
            "RSX_ME_QTY_DECIMALS": str(sym["qty_dec"]),
            "RSX_ME_TICK_SIZE": str(sym["tick"]),
            "RSX_ME_LOT_SIZE": str(sym["lot"]),
            "RSX_ME_WAL_DIR": f"./tmp/wal/{sym['name'].lower()}",
            "RSX_ME_CMP_ADDR": me_cmp,
            "RSX_RISK_CMP_ADDR": risk_cmp,
            "RSX_MD_CMP_ADDR": md_cmp,
            "RSX_ME_DATABASE_URL": pg_url,
            "RUST_LOG": "info",
        }
        plan.append((
            f"me-{sym['name'].lower()}",
            f"{target}/rsx-matching",
            env,
        ))

    # ── Mark aggregator ─────────────────────────────
    plan.append((
        "mark",
        f"{target}/rsx-mark",
        {
            "RSX_MARK_LISTEN_ADDR":
                f"127.0.0.1:{BASE_MARK_CMP}",
            "RSX_MARK_WAL_DIR": "./tmp/wal/mark",
            "RSX_MARK_STREAM_ID": "100",
            "RSX_MARK_SYMBOL_MAP": mark_symbol_map,
            "RSX_RISK_MARK_CMP_ADDR":
                f"127.0.0.1:{BASE_RISK_MARK_CMP}",
            "RUST_LOG": "info",
        },
    ))

    # ── Risk engine ─────────────────────────────────
    for shard in range(config["risk_shards"]):
        # primary
        plan.append((
            f"risk-{shard}",
            f"{target}/rsx-risk",
            {
                "RSX_RISK_SHARD_ID": str(shard),
                "RSX_RISK_SHARD_COUNT":
                    str(config["risk_shards"]),
                "RSX_RISK_MAX_SYMBOLS": str(len(symbols)),
                "RSX_RISK_CMP_ADDR":
                    f"127.0.0.1:{BASE_RISK_CMP + shard}",
                "RSX_GW_CMP_ADDR":
                    f"127.0.0.1:{BASE_GW_CMP}",
                "RSX_ME_CMP_ADDR":
                    f"127.0.0.1:{BASE_ME_CMP + symbols[0]['id']}",
                "RSX_RISK_WAL_DIR": "./tmp/wal",
                "RSX_RISK_MARK_CMP_ADDR":
                    f"127.0.0.1:{BASE_RISK_MARK_CMP}",
                "RSX_MARK_CMP_ADDR":
                    f"127.0.0.1:{BASE_MARK_CMP}",
                "DATABASE_URL": pg_url,
                "RUST_LOG": "info",
            },
        ))

        # replicas
        if replication in ("replica", "spare"):
            replica_count = 2 if replication == "spare" else 1
            for r in range(replica_count):
                plan.append((
                    f"risk-{shard}-replica-{r}",
                    f"{target}/rsx-risk",
                    {
                        "RSX_RISK_SHARD_ID": str(shard),
                        "RSX_RISK_SHARD_COUNT":
                            str(config["risk_shards"]),
                        "RSX_RISK_MAX_SYMBOLS":
                            str(len(symbols)),
                        "RSX_RISK_IS_REPLICA": "true",
                        "RSX_RISK_CMP_ADDR":
                            f"127.0.0.1:{BASE_RISK_CMP + 10 + shard * 2 + r}",
                        "RSX_GW_CMP_ADDR":
                            f"127.0.0.1:{BASE_GW_CMP}",
                        "RSX_ME_CMP_ADDR":
                            f"127.0.0.1:{BASE_ME_CMP + symbols[0]['id']}",
                        "RSX_RISK_WAL_DIR": "./tmp/wal",
                        "DATABASE_URL": pg_url,
                        "RUST_LOG": "info",
                    },
                ))

    # ── Gateway(s) ──────────────────────────────────
    for gw in range(config["gateways"]):
        # per-symbol tick/lot env vars for gateway
        sym_env = {}
        for sym in symbols:
            sid = sym["id"]
            sym_env[f"RSX_SYMBOL_{sid}_TICK_SIZE"] = str(
                sym["tick"]
            )
            sym_env[f"RSX_SYMBOL_{sid}_LOT_SIZE"] = str(
                sym["lot"]
            )
        plan.append((
            f"gw-{gw}",
            f"{target}/rsx-gateway",
            {
                "RSX_GW_LISTEN":
                    f"0.0.0.0:{BASE_GW_WS + gw}",
                "RSX_GW_CMP_ADDR":
                    f"127.0.0.1:{BASE_GW_CMP + gw}",
                "RSX_RISK_CMP_ADDR":
                    f"127.0.0.1:{BASE_RISK_CMP}",
                "RSX_GW_WAL_DIR": "./tmp/wal",
                "RSX_GW_JWT_SECRET":
                    "dev-secret-change-in-production",
                "RSX_MAX_SYMBOLS": "16",
                **sym_env,
                "RUST_LOG": "info",
            },
        ))

    # ── Marketdata ──────────────────────────────────
    plan.append((
        "marketdata",
        f"{target}/rsx-marketdata",
        {
            "RSX_MD_LISTEN":
                f"0.0.0.0:{BASE_MD_WS}",
            "RSX_MKT_CMP_ADDR":
                f"127.0.0.1:{BASE_MD_CMP + symbols[0]['id']}",
            "RSX_ME_CMP_ADDR":
                f"127.0.0.1:{BASE_ME_CMP + symbols[0]['id']}",
            "RSX_MD_STREAM_ID": "1",
            "RUST_LOG": "info",
        },
    ))

    return plan


def print_plan(plan):
    """Print spawn plan without running."""
    print("spawn plan:")
    print(f"  {'name':<24} {'binary':<36} env vars")
    print("  " + "-" * 72)
    for name, binary, env in plan:
        short_bin = binary.split("/")[-1]
        env_summary = " ".join(
            f"{k}={v}" for k, v in sorted(env.items())
            if k != "RUST_LOG"
        )
        # truncate env summary for display
        if len(env_summary) > 80:
            env_summary = env_summary[:77] + "..."
        print(f"  {name:<24} {short_bin:<36} {env_summary}")
    print(f"\n  total: {len(plan)} processes")


# ── process management ──────────────────────────────────

CHILDREN = []


def spawn(name, binary, env):
    """Spawn a process with prefixed output."""
    full_env = {**os.environ, **env}
    proc = subprocess.Popen(
        [binary],
        env=full_env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    CHILDREN.append((name, proc))
    # background reader thread for prefixed output
    import threading

    def reader():
        prefix = f"[{name}]"
        for line in iter(proc.stdout.readline, b""):
            text = line.decode("utf-8", errors="replace")
            print(f"{prefix} {text}", end="", flush=True)

    t = threading.Thread(target=reader, daemon=True)
    t.start()
    return proc


def shutdown_all():
    """SIGTERM all children and wait."""
    print("\nshutting down...")
    for name, proc in CHILDREN:
        if proc.poll() is None:
            proc.terminate()
    deadline = time.time() + 5.0
    for name, proc in CHILDREN:
        remaining = max(0, deadline - time.time())
        try:
            proc.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            print(f"  killing {name}")
            proc.kill()
    print("all processes stopped")


def ensure_dirs(symbols):
    """Create required directories."""
    dirs = [
        "./tmp/wal/mark",
        "./tmp/snapshot",
        "./log",
    ]
    for sym in symbols:
        dirs.append(
            f"./tmp/wal/{sym['name'].lower()}"
        )
    for d in dirs:
        os.makedirs(d, exist_ok=True)


def ensure_postgres_docker(pg_url):
    """Start or reuse a named Postgres container."""
    # check if container exists and is running
    result = subprocess.run(
        ["docker", "inspect", "-f",
         "{{.State.Running}}", PG_CONTAINER],
        capture_output=True, text=True,
    )
    if result.returncode == 0:
        if "true" in result.stdout:
            print(f"reusing postgres container {PG_CONTAINER}")
            return
        # exists but stopped, start it
        subprocess.run(
            ["docker", "start", PG_CONTAINER],
            check=True,
        )
        print(f"started existing postgres "
              f"container {PG_CONTAINER}")
        time.sleep(2)
        return

    # create new container
    print(f"creating postgres container {PG_CONTAINER}")
    subprocess.run([
        "docker", "run", "-d",
        "--name", PG_CONTAINER,
        "-e", "POSTGRES_USER=rsx",
        "-e", "POSTGRES_PASSWORD=rsx",
        "-e", "POSTGRES_DB=rsx",
        "-p", "5432:5432",
        "postgres:16-alpine",
    ], check=True)
    # wait for postgres to be ready
    print("waiting for postgres...")
    for _ in range(30):
        time.sleep(1)
        r = subprocess.run(
            ["docker", "exec", PG_CONTAINER,
             "pg_isready", "-U", "rsx"],
            capture_output=True,
        )
        if r.returncode == 0:
            print("postgres ready")
            return
    print("error: postgres did not become ready")
    sys.exit(1)


def run_migration(pg_url):
    """Run SQL migration via psql."""
    migration = os.path.join(
        os.path.dirname(__file__),
        "rsx-risk/migrations/001_base_schema.sql",
    )
    if not os.path.exists(migration):
        print(f"warning: migration not found: {migration}")
        return
    result = subprocess.run(
        ["psql", pg_url, "-f", migration],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        print(f"migration warning: {result.stderr.strip()}")
    else:
        print("migration applied")


# ── main ────────────────────────────────────────────────


def main():
    parser = argparse.ArgumentParser(
        description="RSX local dev runner",
    )
    parser.add_argument(
        "scenario", nargs="?", default="minimal",
        help="scenario name or code (default: minimal)",
    )
    parser.add_argument(
        "--postgres", default="external",
        choices=["docker", "external"],
        help="postgres mode (default: external)",
    )
    parser.add_argument("--pg-url", default=DEFAULT_PG_URL)
    parser.add_argument(
        "--build", action="store_true",
        help="run cargo build first",
    )
    parser.add_argument(
        "--release", action="store_true",
        help="build in release mode",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="print spawn plan without running",
    )
    parser.add_argument("--symbols", default=None)
    parser.add_argument(
        "--gateways", type=int, default=None,
    )
    parser.add_argument(
        "--risk-shards", type=int, default=None,
    )
    parser.add_argument(
        "--replication", default=None,
        choices=["none", "replica", "spare"],
    )

    args = parser.parse_args()
    config = resolve_scenario(args)
    plan = build_spawn_plan(config, args.pg_url, args.release)

    if args.dry_run:
        print_plan(plan)
        return

    # build
    if args.build:
        cmd = ["cargo", "build"]
        if args.release:
            cmd.append("--release")
        print(f"building: {' '.join(cmd)}")
        result = subprocess.run(cmd)
        if result.returncode != 0:
            print("build failed")
            sys.exit(1)

    # dirs
    ensure_dirs(config["symbols"])

    # postgres
    if args.postgres == "docker":
        ensure_postgres_docker(args.pg_url)

    # migration
    run_migration(args.pg_url)

    # spawn
    print_plan(plan)
    print()

    signal.signal(
        signal.SIGINT,
        lambda *_: shutdown_all() or sys.exit(0),
    )
    signal.signal(
        signal.SIGTERM,
        lambda *_: shutdown_all() or sys.exit(0),
    )

    for name, binary, env in plan:
        if not os.path.exists(binary):
            print(f"error: binary not found: {binary}")
            print("run with --build to compile first")
            shutdown_all()
            sys.exit(1)
        print(f"  starting {name}...")
        spawn(name, binary, env)
        time.sleep(0.1)  # stagger startup

    print(f"\n{len(CHILDREN)} processes running. "
          f"ctrl-c to stop.\n")

    # wait for any child to exit
    try:
        while True:
            for name, proc in CHILDREN:
                ret = proc.poll()
                if ret is not None:
                    print(f"  {name} exited with {ret}")
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    finally:
        shutdown_all()

    # summary
    print("\nexit summary:")
    for name, proc in CHILDREN:
        code = proc.returncode or 0
        status = "ok" if code == 0 else f"exit {code}"
        print(f"  {name}: {status}")


if __name__ == "__main__":
    main()
