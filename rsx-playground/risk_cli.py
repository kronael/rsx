"""Symbol config, accounts and risk reads through `rsx-cli config|account|risk … --json`.

Each call is one finite CLI run against risk's Postgres. `rsx_cli.CliError`
carries the CLI's own error line; code 2 means the CLI refused the change
(a margin rule, a value out of range). The CLI applies risk's migrations
before every command.
"""

from dataclasses import dataclass
from typing import Any

import rsx_cli

EXIT_MISUSE = 2
SEED_TIMEOUT_S = 30.0
NO_ACCOUNT = "has no account"
USER_ID_MAX = 2**32 - 1


class NoAccount(rsx_cli.CliError):
    pass


@dataclass(frozen=True)
class ConfigColumns:
    symbol_id: int
    config_version: int
    effective_at_ms: int
    tick_size: int
    lot_size: int
    price_decimals: int
    qty_decimals: int
    status: str
    min_notional: int | None
    max_order_qty: int | None
    maker_fee_bps: int | None
    taker_fee_bps: int | None
    initial_margin_rate_bps: int | None
    maintenance_margin_rate_bps: int | None
    max_leverage: int | None
    funding_interval_sec: int | None
    funding_rate_min_bps: int | None
    funding_rate_max_bps: int | None


@dataclass(frozen=True)
class ConfigRow(ConfigColumns):
    created_at_ms: int


@dataclass(frozen=True)
class AppliedRow(ConfigColumns):
    applied_at_ns: int


@dataclass(frozen=True)
class SymbolConfig:
    symbol_id: int
    name: str | None
    active_version: int | None
    scheduled: list[ConfigRow]
    applied: AppliedRow | None

    @classmethod
    def of(cls, data: dict[str, Any]) -> "SymbolConfig":
        applied = data["applied"]
        return cls(
            symbol_id=data["symbol_id"],
            name=data["name"],
            active_version=data["active_version"],
            scheduled=[ConfigRow(**row) for row in data["scheduled"]],
            applied=None if applied is None else AppliedRow(**applied),
        )

    def active(self) -> ConfigRow | None:
        return next(
            (
                row
                for row in self.scheduled
                if row.config_version == self.active_version
            ),
            None,
        )


@dataclass(frozen=True)
class Scheduled:
    symbol_id: int
    config_version: int
    effective_at_ms: int


@dataclass(frozen=True)
class Seeded:
    accounts: int
    collateral: int


@dataclass(frozen=True)
class Position:
    symbol_id: int
    net_qty: int
    avg_px: int
    upnl: int | None


@dataclass(frozen=True)
class Frozen:
    order_id: str
    symbol_id: int
    amount: int


@dataclass(frozen=True)
class Account:
    user_id: int
    collateral: int
    positions: list[Position]
    frozen: list[Frozen]

    @classmethod
    def of(cls, data: dict[str, Any]) -> "Account":
        return cls(
            user_id=data["user_id"],
            collateral=data["collateral"],
            positions=[Position(**item) for item in data["positions"]],
            frozen=[Frozen(**item) for item in data["frozen"]],
        )

    def frozen_amount(self) -> int:
        return sum(order.amount for order in self.frozen)


@dataclass(frozen=True)
class Fund:
    symbol_id: int
    balance: int
    version: int


@dataclass(frozen=True)
class Overview:
    users: list[Account]
    funds: list[Fund]
    insurance_total: int
    liquidations: list[dict[str, Any]]
    marks: dict[int, int]

    @classmethod
    def of(cls, data: dict[str, Any], marks: dict[int, int]) -> "Overview":
        insurance = data["insurance"]
        return cls(
            users=[Account.of(item) for item in data["users"]],
            funds=[Fund(**item) for item in insurance["funds"]],
            insurance_total=insurance["total"],
            liquidations=list(data["liquidations"]),
            marks=marks,
        )


def mark_args(marks: dict[int, int]) -> list[str]:
    args: list[str] = []
    for symbol_id, px in sorted(marks.items()):
        args += ["--mark", f"{symbol_id}={px}"]
    return args


async def config_show(database_url: str) -> list[SymbolConfig]:
    data = await rsx_cli.run("config", "show", "--db", database_url, "--json")
    return [SymbolConfig.of(item) for item in data["symbols"]]


async def config_schedule(
    database_url: str,
    symbol_id: int,
    settings: dict[str, int],
    effective_at_ms: int,
) -> Scheduled:
    args = ["config", "schedule", "-s", str(symbol_id)]
    for column, value in settings.items():
        args += ["--set", f"{column}={value}"]
    args += ["--effective-at-ms", str(effective_at_ms)]
    data = await rsx_cli.run(*args, "--db", database_url, "--json")
    return Scheduled(**data)


async def account_seed(
    database_url: str, count: int | None = None, collateral: int | None = None
) -> Seeded:
    args = ["account", "seed"]
    if count is not None:
        args += ["--count", str(count)]
    if collateral is not None:
        args += ["--collateral", str(collateral)]
    data = await rsx_cli.run(
        *args, "--db", database_url, "--json", timeout=SEED_TIMEOUT_S
    )
    return Seeded(**data)


async def account_show(
    database_url: str, user_id: int, marks: dict[int, int]
) -> Account:
    if not 0 <= user_id <= USER_ID_MAX:
        raise NoAccount(f"user {user_id} {NO_ACCOUNT}", 1)
    args = ["account", "show", "-u", str(user_id), *mark_args(marks)]
    try:
        data = await rsx_cli.run(*args, "--db", database_url, "--json")
    except rsx_cli.CliError as err:
        if NO_ACCOUNT in str(err):
            raise NoAccount(str(err), err.code) from err
        raise
    return Account.of(data)


async def risk_overview(database_url: str, marks: dict[int, int]) -> Overview:
    args = ["risk", "overview", *mark_args(marks)]
    data = await rsx_cli.run(*args, "--db", database_url, "--json")
    return Overview.of(data, marks)
