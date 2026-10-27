"""The symbols risk has in force, as `config show` reports them.

The server refreshes `table` from the CLI; the page renderers read it. No
symbol is known before the first refresh, and one the table lacks shows as
`sym-<id>` with raw, unscaled numbers.
"""

from dataclasses import dataclass

import risk_cli


@dataclass(frozen=True)
class Symbol:
    id: int
    name: str
    tick: int
    lot: int
    price_dec: int
    qty_dec: int


table: dict[str, Symbol] = {}


def from_configs(configs: list[risk_cli.SymbolConfig]) -> dict[str, Symbol]:
    """The named symbols with an active config row, by name."""
    found = {}
    for config in configs:
        row = config.active()
        if row is None or config.name is None:
            continue
        found[config.name] = Symbol(
            config.symbol_id,
            config.name,
            row.tick_size,
            row.lot_size,
            row.price_decimals,
            row.qty_decimals,
        )
    return found


async def read(database_url: str) -> dict[int, Symbol]:
    """The symbols risk has in force, by id, straight from `config show`."""
    configs = await risk_cli.config_show(database_url)
    return {symbol.id: symbol for symbol in from_configs(configs).values()}


def by_id(symbol_id: int) -> Symbol | None:
    return next(
        (symbol for symbol in table.values() if symbol.id == symbol_id),
        None,
    )


def name_of(symbol_id: int) -> str:
    symbol = by_id(symbol_id)
    return symbol.name if symbol else f"sym-{symbol_id}"


def price_decimals(symbol_id: int) -> int:
    symbol = by_id(symbol_id)
    return symbol.price_dec if symbol else 0


def qty_decimals(symbol_id: int) -> int:
    symbol = by_id(symbol_id)
    return symbol.qty_dec if symbol else 0
