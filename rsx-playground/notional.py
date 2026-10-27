"""Minimum order notional: raw risk units to USD and back.

Raw is risk's `price x qty` unit, 10^-(price_decimals + qty_decimals) USD,
so 1 USD is 10^(price_decimals + qty_decimals). Postgres and the wire hold
the raw integer. Decimal exists only here, at display and at entry; no
float touches a stored value.
"""

from decimal import ROUND_HALF_UP
from decimal import Decimal
from decimal import localcontext

I64_MAX = 2**63 - 1


def usd_from_raw(raw: int, price_decimals: int, qty_decimals: int) -> str:
    """Exact USD text of a raw minimum, at least two decimals."""
    places = price_decimals + qty_decimals
    text = format(Decimal(raw).scaleb(-places).normalize(), "f")
    whole, _, fraction = text.partition(".")
    return f"{whole}.{fraction.ljust(2, '0')}"


def parse_usd(text: str) -> Decimal:
    """A positive finite USD amount, or ValueError with the reason."""
    try:
        usd = Decimal(text.strip())
    except ArithmeticError:
        raise ValueError(
            "minimum order must be a number of USD") from None
    if not usd.is_finite() or usd <= 0:
        raise ValueError("minimum order must be a positive number of USD")
    return usd


def raw_from_usd(usd: Decimal, price_decimals: int, qty_decimals: int) -> int:
    """Raw integer of a positive USD amount, rounded half up.

    ValueError when it rounds to zero (risk refuses a zero minimum) or
    does not fit the BIGINT column.
    """
    places = price_decimals + qty_decimals
    too_large = ValueError(
        f"minimum order is too large (raw value above {I64_MAX})")
    if usd > I64_MAX:
        raise too_large
    with localcontext() as context:
        context.prec = 60
        raw = int(usd.scaleb(places).quantize(
            Decimal(1), rounding=ROUND_HALF_UP))
    if raw < 1:
        raise ValueError(
            "minimum order is smaller than the smallest unit "
            f"(1e-{places} USD)")
    if raw > I64_MAX:
        raise too_large
    return raw


def one_usd_raw(price_decimals: int, qty_decimals: int) -> int:
    """1 USD in raw units: the minimum the presets carry."""
    return 10 ** (price_decimals + qty_decimals)


def catalog_min_notional(row: dict) -> int:
    """The minimum order notional of a `/v1/symbols` row, in raw units.

    The row's own `min_notional` wins. The catalog omits it while its
    config source is down; the row's decimals then give the 1 USD preset
    minimum. 0 when neither is present (no floor known).
    """
    raw = row.get("min_notional")
    if isinstance(raw, int) and raw > 0:
        return raw
    price_decimals = row.get("price_decimals")
    qty_decimals = row.get("qty_decimals")
    if isinstance(price_decimals, int) and isinstance(qty_decimals, int):
        return one_usd_raw(price_decimals, qty_decimals)
    return 0


def min_order_qty(min_notional: int, price: int, lot: int) -> int:
    """Smallest quantity, a multiple of `lot`, with `price * qty` at or
    above `min_notional` (risk's check, in raw price x qty units). At
    least one lot."""
    if price <= 0 or lot <= 0:
        raise ValueError("price and lot must be positive")
    lots = -(-max(min_notional, 0) // (price * lot))
    return max(lots, 1) * lot


def size_qty(qty: int, price: int, min_notional: int, lot: int) -> int:
    """`qty` rounded up to a lot multiple and raised to the minimum order
    quantity at `price`."""
    if lot <= 0:
        raise ValueError("lot must be positive")
    aligned = -(-qty // lot) * lot
    return max(aligned, min_order_qty(min_notional, price, lot))

