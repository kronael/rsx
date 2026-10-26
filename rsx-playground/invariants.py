"""The Verify page's data: the `rsx-cli verify --json` report, parsed."""

import time
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any

import rsx_cli

VERIFY_TIMEOUT_S = 60.0


class Status(StrEnum):
    PASS = "pass"
    FAIL = "fail"
    SKIP = "skip"


@dataclass(frozen=True)
class Check:
    invariant: int | None
    name: str
    status: Status
    detail: str
    checked: int
    violation_count: int
    violations: list[dict[str, Any]]


@dataclass(frozen=True)
class Report:
    status: Status
    checks: list[Check]
    run_at_ms: int
    error: str = ""

    def failed_checks(self) -> list[Check]:
        return [check for check in self.checks if check.status is Status.FAIL]


def failed_report(error: str) -> Report:
    return Report(Status.FAIL, [], int(time.time() * 1000), error)


def parse_report(text: str) -> Report:
    raw = rsx_cli.parse_line(text)
    try:
        checks = [
            Check(**{**check, "status": Status(check["status"])})
            for check in raw["checks"]
        ]
        status = Status(raw["status"])
    except (KeyError, TypeError, ValueError) as err:
        raise rsx_cli.CliError(f"unexpected rsx-cli verify report: {err!r}") from err
    return Report(status, checks, int(time.time() * 1000))


async def run_verify(wal_dirs: list[Path], db_url: str) -> Report:
    """`rsx-cli verify --json` over the WAL directories and Postgres.

    A failed check exits 1 with the full report on stdout; that report is
    returned. Any other failure raises CliError with the CLI's reason.
    """
    args = ["verify", "--json", "--db", db_url, *map(str, wal_dirs)]
    try:
        text = await rsx_cli.run_text(*args, timeout=VERIFY_TIMEOUT_S)
    except rsx_cli.CliError as err:
        if not err.stdout:
            raise
        text = err.stdout
    return parse_report(text)


@dataclass
class Verifier:
    """The last report and whether a run is in flight."""

    report: Report | None = None
    running: bool = False

    def is_fresh(self, max_age_s: float) -> bool:
        if self.report is None:
            return False
        return time.time() * 1000 - self.report.run_at_ms < max_age_s * 1000

    async def run(self, wal_dirs: list[Path], db_url: str) -> Report:
        """Runs the checks and stores the report; a CliError becomes a failed report."""
        self.running = True
        try:
            report = await run_verify(wal_dirs, db_url)
        except rsx_cli.CliError as err:
            report = failed_report(str(err))
        finally:
            self.running = False
        self.report = report
        return report
