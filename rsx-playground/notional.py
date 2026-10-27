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
