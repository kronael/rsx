"""The exchange processes through `rsx-cli cluster … --json`.

The CLI starts every daemon detached, so the playground server does not own
them: it asks `ps` what runs and tells `up`, `down`, `restart` and `reset` what
to do. `rsx_cli.CliError` carries the CLI's own error line; code 2 means the
arguments were wrong, such as a process the scenario does not have.
RSX_PLAYGROUND_RELEASE=1 runs the release binaries.
"""

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import rsx_cli

ROOT = rsx_cli.ROOT
READY_WAIT_S = 120
BUILD_TIMEOUT_S = 1800.0
CALL_MARGIN_S = 30.0
EXIT_MISUSE = 2


@dataclass(frozen=True)
class Row:
    name: str
    pid: int | None
    state: str
    started_ms: int | None
    log: str


@dataclass(frozen=True)
class View:
    scenario: str | None
    rows: list[Row]

    def row(self, name: str) -> Row | None:
        return next((row for row in self.rows if row.name == name), None)

    def is_running(self, name: str) -> bool:
        row = self.row(name)
        return row is not None and row.state == "running"


@dataclass(frozen=True)
class Skipped:
    name: str
    reason: str


@dataclass(frozen=True)
class Report:
    started: list[str]
    stopped: list[str]
    skipped: list[Skipped]
    cleared: list[str]

    @classmethod
    def of(cls, data: dict[str, Any]) -> "Report":
        return cls(
            started=list(data.get("started", [])),
            stopped=list(data.get("stopped", [])),
            skipped=[Skipped(**item) for item in data.get("skipped", [])],
            cleared=list(data.get("cleared", [])),
        )

    def is_skipped(self, name: str) -> bool:
        return any(skipped.name == name for skipped in self.skipped)


@dataclass(frozen=True)
class Scenario:
    name: str
    symbols: int
    gateways: int
    replication: str
    load: dict[str, Any] | None


@dataclass(frozen=True)
class PlanProcess:
    name: str
    binary: str
    env: dict[str, str]


def release_args() -> list[str]:
    return ["-r"] if os.environ.get("RSX_PLAYGROUND_RELEASE") == "1" else []


def args_for(action: str, *args: str) -> list[str]:
    if action in ("plan", "up", "restart"):
        args = (*release_args(), *args)
    return ["cluster", action, "-C", str(ROOT), *args, "--json"]


async def scenarios() -> list[Scenario]:
    data = await rsx_cli.run("cluster", "scenarios", "--json")
    return [Scenario(**item) for item in data["scenarios"]]


async def plan(scenario: str) -> list[PlanProcess]:
    data = await rsx_cli.run(*args_for("plan", "-s", scenario), timeout=30.0)
    return [
        PlanProcess(item["name"], item["binary"], item["env"])
        for item in data["processes"]
    ]


async def ps() -> View:
    data = await rsx_cli.run(*args_for("ps"))
    return View(data["scenario"], [Row(**item) for item in data["processes"]])


async def up(
    scenario: str, only: tuple[str, ...] = (), build: bool = False
) -> Report:
    args = ["-s", scenario, "-w", str(READY_WAIT_S)]
    for name in only:
        args += ["-o", name]
    if build:
        args.append("-b")
    timeout = BUILD_TIMEOUT_S if build else READY_WAIT_S + CALL_MARGIN_S
    data = await rsx_cli.run(*args_for("up", *args), timeout=timeout)
    return Report.of(data)


async def down(only: tuple[str, ...] = ()) -> Report:
    args: list[str] = []
    for name in only:
        args += ["-o", name]
    data = await rsx_cli.run(*args_for("down", *args), timeout=60.0)
    return Report.of(data)


async def restart(name: str, scenario: str) -> Report:
    args = [name, "-s", scenario, "-w", str(READY_WAIT_S)]
    data = await rsx_cli.run(
        *args_for("restart", *args), timeout=READY_WAIT_S + CALL_MARGIN_S
    )
    return Report.of(data)


async def reset() -> Report:
    data = await rsx_cli.run(*args_for("reset"), timeout=60.0)
    return Report.of(data)


def binary_path(binary: str) -> Path:
    path = Path(binary)
    return path if path.is_absolute() else ROOT / binary
