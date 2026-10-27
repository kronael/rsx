"""Stress runs and latency probes through `rsx-cli stress`.

A load run is a managed child of the server: `Stress` starts it, stops it, and
keeps the pid and the last error. The CLI sends the orders and writes its JSON
report (`rsx-cli/README.md` § stress) to `stress-<id>.json` in the reports
directory; the pages read that file as it is. A probe is one finite call that
measures a single order.
"""

import asyncio
import json
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

import rsx_cli

START_WAIT_S = 1.0
PID_FILE = rsx_cli.PID_DIR / "stress.pid"
PROBE_TIMEOUT_MS = 2000
PROBE_MARGIN_S = 10.0


@dataclass(frozen=True)
class Target:
    """Where the CLI sends orders and reads prices."""

    gateway: str
    marketdata: str
    database_url: str

    def args(self) -> list[str]:
        return [
            "-g",
            self.gateway,
            "--md",
            self.marketdata,
            "--db",
            self.database_url,
        ]


@dataclass(frozen=True)
class Load:
    rate: int
    duration_s: int
    users: int = 10

    def args(self) -> list[str]:
        return [
            "-m",
            "load",
            "-r",
            str(self.rate),
            "-d",
            str(self.duration_s),
            "-u",
            str(self.users),
        ]


@dataclass(frozen=True)
class Summary:
    """A report as one row of the reports list."""

    id: str
    timestamp: str
    rate: int
    duration: int
    submitted: int
    accepted: int
    accept_rate: float
    p99_latency: int | None
    status: str
    failures: list[str]


def accept_rate(report: dict[str, Any]) -> float:
    submitted = report["submitted"]
    if not submitted:
        return 0.0
    return round(100 * report["accepted"] / submitted, 1)


def summarize(report_id: str, report: dict[str, Any]) -> Summary:
    config = report["config"]
    return Summary(
        id=report_id,
        timestamp=report_id,
        rate=config["target_rate"],
        duration=config["duration_s"],
        submitted=report["submitted"],
        accepted=report["accepted"],
        accept_rate=accept_rate(report),
        p99_latency=report["latency_us"]["p99"],
        status=report["status"],
        failures=list(report["failures"]),
    )


class Stress:
    """The load run of this server: at most one child, its last outcome."""

    def __init__(self, report_dir: Path, log_dir: Path) -> None:
        self.report_dir = report_dir
        self.log_dir = log_dir
        self.task: asyncio.Task[None] | None = None
        self.pid: int | None = None
        self.report: Path | None = None
        self.error: str | None = None

    def is_running(self) -> bool:
        return self.task is not None and not self.task.done()

    async def start(self, load: Load, target: Target) -> None:
        """Starts the child; raises CliError when it does not stay up.

        A child that exits within START_WAIT_S without a report (a gateway
        that refuses the token, a missing secret) is a failed start, and the
        CLI's own error line is the message.
        """
        if self.is_running():
            raise rsx_cli.CliError("stress already running")
        self.report_dir.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        self.report = self.report_dir / f"stress-{stamp}.json"
        self.pid = None
        self.error = None
        args = ["stress", *load.args(), *target.args(), "-o", str(self.report)]
        self.task = asyncio.create_task(self.supervise(args), name="stress")
        await asyncio.wait({self.task}, timeout=START_WAIT_S)
        if self.task.done() and self.error is not None:
            raise rsx_cli.CliError(self.error)

    async def stop(self) -> None:
        """SIGTERM the child; the CLI finishes in-flight orders and reports."""
        if self.task is None:
            return
        self.task.cancel()
        await asyncio.wait({self.task})

    async def supervise(self, args: list[str]) -> None:
        assert self.report is not None
        try:
            async with rsx_cli.spawn(*args, pid_file=PID_FILE) as proc:
                self.pid = proc.pid
                out, err = await proc.communicate()
        except rsx_cli.CliError as exc:
            self.error = str(exc)
            return
        stderr = err.decode(errors="replace")
        self.log_dir.mkdir(parents=True, exist_ok=True)
        log = out.decode(errors="replace") + stderr
        (self.log_dir / "stress.log").write_text(log)
        if proc.returncode != 0 and not self.report.exists():
            self.error = rsx_cli.error_message(stderr)


@dataclass(frozen=True)
class Probe:
    """One order, measured by the CLI; `error` is set when it was not."""

    elapsed_us: int | None
    reasons: dict[str, int]
    error: str | None


async def probe(target: Target, mode: str, symbol_id: int | None = None) -> Probe:
    """One order of `mode` ("probe": a crossing fill; "reject": the
    gateway's own reject) measured by `rsx-cli stress`."""
    args = ["stress", "-m", mode, "-n", "1", "-w", "0"]
    if symbol_id is not None:
        args += ["-s", str(symbol_id)]
    args += ["-t", str(PROBE_TIMEOUT_MS), *target.args(), "--json"]
    timeout = PROBE_TIMEOUT_MS / 1000 + PROBE_MARGIN_S
    try:
        text = await rsx_cli.run_text(*args, timeout=timeout)
    except rsx_cli.CliError as err:
        if err.code != 1 or not err.stdout.strip():
            return Probe(None, {}, str(err))
        text = err.stdout
    try:
        report = rsx_cli.parse_line(text)
    except rsx_cli.CliError as err:
        return Probe(None, {}, str(err))
    elapsed_us = report["latency_us"]["p50"]
    if elapsed_us is None:
        return Probe(None, {}, "; ".join(report["failures"]) or "no sample")
    return Probe(elapsed_us, dict(report["rejected_by_reason"]), None)


def read_report(path: Path) -> dict[str, Any]:
    with open(path) as fp:
        report = json.load(fp)
    if not isinstance(report, dict):
        raise TypeError("report is not an object")
    return report
