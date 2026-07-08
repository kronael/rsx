"""Decoder for the rsx-marketdata protobuf feed (pure-python, no deps).

The public market-data feed sends protobuf ``MdFrame`` messages as
WebSocket BINARY frames. The shared wire schema lives in
``rsx-marketdata/marketdata.proto``; the Rust producer hand-derives it
in ``rsx-marketdata/src/wire.rs``. This module hand-rolls a matching
varint decoder so the dashboard needs no ``protobuf`` runtime or
``protoc`` codegen.

Each frame on the wire is ``[len:u32 big-endian][MdFrame body]``.
:func:`decode` strips the prefix and returns the SAME dict shape the
old JSON feed produced, so downstream consumers are unchanged:

    {"BBO": [sym, bid_px, bid_qty, bid_cnt, ask_px, ask_qty, ask_cnt, ts, seq]}
    {"B":   [sym, [[px, qty, cnt], ...], [[px, qty, cnt], ...], ts, seq]}
    {"D":   [sym, side, px, qty, cnt, ts, seq]}
    {"T":   [sym, px, qty, taker_side, ts, seq]}
    {"H":   [ts_ms]}

proto3 omits zero-valued scalars, so an absent field decodes as 0.
"""

_MASK64 = (1 << 64) - 1


def _read_varint(buf: bytes, i: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while True:
        b = buf[i]
        i += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            return result, i
        shift += 7


def _as_i64(v: int) -> int:
    """Interpret a varint as a two's-complement signed int64."""
    v &= _MASK64
    return v - (1 << 64) if v >= (1 << 63) else v


def _fields(buf: bytes) -> dict[int, list]:
    """Parse a protobuf message body into ``{field_number: [values]}``.

    Varint fields (wire type 0) yield ints; length-delimited fields
    (wire type 2) yield ``bytes``. The marketdata schema uses only
    those two wire types.
    """
    out: dict[int, list] = {}
    i = 0
    n = len(buf)
    while i < n:
        key, i = _read_varint(buf, i)
        field_number = key >> 3
        wire_type = key & 0x7
        if wire_type == 0:
            val, i = _read_varint(buf, i)
        elif wire_type == 2:
            length, i = _read_varint(buf, i)
            val = buf[i:i + length]
            i += length
        else:
            raise ValueError(f"unsupported wire type {wire_type}")
        out.setdefault(field_number, []).append(val)
    return out


def _u(f: dict[int, list], tag: int) -> int:
    """Last unsigned scalar for ``tag``, or 0 (proto3 default) if absent."""
    vs = f.get(tag)
    return vs[-1] if vs else 0


def _levels(f: dict[int, list], tag: int) -> list[list[int]]:
    out = []
    for raw in f.get(tag, []):
        lf = _fields(raw)
        out.append([_as_i64(_u(lf, 1)), _as_i64(_u(lf, 2)), _u(lf, 3)])
    return out


def decode(data: bytes) -> dict | None:
    """Decode one WS BINARY frame to a feed dict, or ``None`` if malformed."""
    if len(data) < 4:
        return None
    body_len = int.from_bytes(data[:4], "big")
    body = data[4:4 + body_len]
    frame = _fields(body)
    # MdFrame is a oneof: exactly one of tags 1..5 is set.
    for tag, payloads in frame.items():
        f = _fields(payloads[-1])
        if tag == 1:  # Bbo
            return {"BBO": [
                _u(f, 1), _as_i64(_u(f, 2)), _as_i64(_u(f, 3)), _u(f, 4),
                _as_i64(_u(f, 5)), _as_i64(_u(f, 6)), _u(f, 7),
                _u(f, 8), _u(f, 9),
            ]}
        if tag == 2:  # Snapshot
            return {"B": [
                _u(f, 1), _levels(f, 2), _levels(f, 3), _u(f, 4), _u(f, 5),
            ]}
        if tag == 3:  # Delta
            return {"D": [
                _u(f, 1), _u(f, 2), _as_i64(_u(f, 3)), _as_i64(_u(f, 4)),
                _u(f, 5), _u(f, 6), _u(f, 7),
            ]}
        if tag == 4:  # Trade
            return {"T": [
                _u(f, 1), _as_i64(_u(f, 2)), _as_i64(_u(f, 3)), _u(f, 4),
                _u(f, 5), _u(f, 6),
            ]}
        if tag == 5:  # Heartbeat
            return {"H": [_u(f, 1)]}
    return None
