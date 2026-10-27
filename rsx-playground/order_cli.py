"""Orders through `rsx-cli order place|cancel|batch --json`.

The CLI owns the gateway client, the token and the order ids; this module
builds its arguments and reads its outcome. A rejected or timed-out order
makes the CLI exit 1 with the outcome on stdout, so that is an `Outcome`,
not an error. `rsx_cli.CliError` is raised when no outcome exists: the
binary is missing, the gateway is down, the token is refused, an argument
is unrepresentable (code 2).
"""

import json
from dataclasses import dataclass
from typing import Any

import rsx_cli

ORDER_TIMEOUT_MS = 3000
EXIT_MISUSE = 2
PROCESS_MARGIN_S = 10.0


@dataclass(frozen=True)
class Outcome:
    cid: str | None
    status: str
    reason: str | None
    order_id: str | None
    rtt_us: int | None
    fills: list[dict[str, Any]]
    frames: list[dict[str, Any]]

    @classmethod
    def of(cls, data: dict[str, Any]) -> "Outcome":
        return cls(
            cid=data.get("cid"),
            status=str(data["status"]),
            reason=data.get("reason"),
            order_id=data.get("order_id"),
            rtt_us=data.get("rtt_us"),
            fills=list(data.get("fills") or []),
            frames=list(data.get("frames") or []),
        )


@dataclass(frozen=True)
class NewOrder:
    user_id: int
    symbol_id: int
    side: str
    tif: str = "gtc"
    price: str | None = None
    px: int | None = None
    qty: str | None = None
    qty_raw: int | None = None
    is_min_size: bool = False
    reduce_only: bool = False
    post_only: bool = False
    cid: str | None = None

    def place_args(self) -> list[str]:
        args = ["-u", str(self.user_id), "-s", str(self.symbol_id)]
        args += ["--side", self.side, "--tif", self.tif.lower()]
        if self.price is not None:
            args += ["-p", self.price]
        if self.px is not None:
            args += ["--px", str(self.px)]
        if self.qty is not None:
            args += ["-q", self.qty]
        if self.qty_raw is not None:
            args += ["--qty-raw", str(self.qty_raw)]
        if self.is_min_size:
            args.append("--min-size")
        if self.reduce_only:
            args.append("--reduce-only")
        if self.post_only:
            args.append("--post-only")
        if self.cid is not None:
            args += ["--cid", self.cid]
        return args

    def batch_line(self) -> str:
        line: dict[str, Any] = {
            "user_id": self.user_id,
            "symbol_id": self.symbol_id,
            "side": self.side,
            "tif": self.tif.lower(),
            "reduce_only": self.reduce_only,
            "post_only": self.post_only,
        }
        if self.price is not None:
            line["price"] = self.price
        if self.px is not None:
            line["px"] = self.px
        if self.qty is not None:
            line["qty"] = self.qty
        if self.qty_raw is not None:
            line["qty_raw"] = self.qty_raw
        if self.is_min_size:
            line["min_size"] = True
        if self.cid is not None:
            line["cid"] = self.cid
        return json.dumps(line)


@dataclass(frozen=True)
class Gateway:
    url: str
    database_url: str | None = None

    def connect_args(self, with_database: bool) -> list[str]:
        args = ["-g", self.url, "-t", str(ORDER_TIMEOUT_MS), "--json"]
        if with_database and self.database_url:
            args += ["--db", self.database_url]
        return args

    async def place(self, order: NewOrder) -> Outcome:
        args = ["order", "place", *order.place_args()]
        args += self.connect_args(order.is_min_size)
        return one_outcome(await run_order(args, 1))

    async def cancel(self, user_id: int, key: str) -> Outcome:
        args = ["order", "cancel", "-u", str(user_id), key, *self.connect_args(False)]
        return one_outcome(await run_order(args, 1))

    async def batch(self, orders: list[NewOrder]) -> list[Outcome]:
        has_min_size = any(order.is_min_size for order in orders)
        args = ["order", "batch", *self.connect_args(has_min_size)]
        stdin = "\n".join(order.batch_line() for order in orders) + "\n"
        return await run_order(args, len(orders), stdin)


async def run_order(
    args: list[str], count: int, stdin: str | None = None
) -> list[Outcome]:
    timeout = count * ORDER_TIMEOUT_MS / 1000 + PROCESS_MARGIN_S
    try:
        text = await rsx_cli.run_text(*args, timeout=timeout, stdin=stdin)
    except rsx_cli.CliError as err:
        if err.code != 1 or not err.stdout.strip():
            raise
        outcomes = parse_outcomes(err.stdout)
        if len(outcomes) < count:
            raise rsx_cli.CliError(
                f"{len(outcomes)} of {count} orders answered before: {err}",
                err.code,
                err.stdout,
            ) from err
        return outcomes
    return parse_outcomes(text)


def parse_outcomes(text: str) -> list[Outcome]:
    return [
        Outcome.of(rsx_cli.parse_line(line))
        for line in text.splitlines()
        if line.strip()
    ]


def one_outcome(outcomes: list[Outcome]) -> Outcome:
    if len(outcomes) != 1:
        raise rsx_cli.CliError(f"expected one order outcome, got {len(outcomes)}")
    return outcomes[0]
