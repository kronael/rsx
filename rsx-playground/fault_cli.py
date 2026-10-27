"""Faults through `rsx-cli fault … --json`.

Each call is one finite CLI run; `rsx_cli.CliError` carries the CLI's own error
line, such as a process with no pid record, a `tc` failure or a missing sudo
rule. The duplicate-order demo runs a scratch matching engine inside the CLI.
"""

from dataclasses import dataclass
from pathlib import Path
from typing import Any

import cluster_cli
import rsx_cli

ROOT = rsx_cli.ROOT
NET_DELAY_MS = 50
NET_LOSS_PCT = 10
NET_TIMEOUT_S = 15.0
DUP_ORDER_TIMEOUT_S = 60.0


@dataclass(frozen=True)
class Killed:
    name: str
    pid: int


@dataclass(frozen=True)
class Net:
    iface: str
    delay_ms: int | None
    loss_pct: float | None
    is_active: bool


@dataclass(frozen=True)
class Flip:
    stream_id: int
    file: Path
    offset: int
    old_byte: int
    new_byte: int


@dataclass(frozen=True)
class DupOrder:
    order_id: str
    records: list[dict[str, Any]]


async def kill(name: str) -> Killed:
    data = await rsx_cli.run("fault", "kill", name, "-C", str(ROOT), "--json")
    return Killed(data["name"], data["pid"])


async def net(is_clear: bool) -> Net:
    args = (
        ["--clear"]
        if is_clear
        else [
            "--delay-ms",
            str(NET_DELAY_MS),
            "--loss-pct",
            str(NET_LOSS_PCT),
        ]
    )
    data = await rsx_cli.run("fault", "net", *args, "--json", timeout=NET_TIMEOUT_S)
    return Net(data["iface"], data["delay_ms"], data["loss_pct"], data["active"])


async def wal_corrupt(stream_id: int, wal_dir: Path) -> Flip:
    data = await rsx_cli.run(
        "fault",
        "wal-corrupt",
        str(stream_id),
        "--wal-dir",
        str(wal_dir),
        "-C",
        str(ROOT),
        "--json",
    )
    return Flip(
        data["stream_id"],
        Path(data["file"]),
        data["offset"],
        data["old_byte"],
        data["new_byte"],
    )


async def dup_order() -> DupOrder:
    data = await rsx_cli.run(
        "fault",
        "dup-order",
        "-C",
        str(ROOT),
        *cluster_cli.release_args(),
        "--json",
        timeout=DUP_ORDER_TIMEOUT_S,
    )
    return DupOrder(data["order_id"], list(data["records"]))
