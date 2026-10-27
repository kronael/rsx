"""The symbols risk has in force, as `config show` reports them.

The server refreshes `table` from the CLI; the page renderers read it. No
symbol is known before the first refresh, and one the table lacks shows as
`sym-<id>` with raw, unscaled numbers.
"""

from dataclasses import dataclass


@dataclass(frozen=True)
class Symbol:
    id: int
    name: str
    tick: int
    lot: int
    price_dec: int
    qty_dec: int


table: dict[str, Symbol] = {}


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
