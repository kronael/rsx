"""Cross-validation of md_wire against the Rust golden bytes.

The byte vectors here are the EXACT frames pinned in
``rsx-marketdata/src/wire_test.rs``. Decoding them with the pure-python
``md_wire`` decoder and asserting the legacy dict shape proves both
implementations agree on the wire schema. If a Rust tag changes, its
golden test flips first; if this decoder drifts, these fail.
"""

import md_wire


def test_bbo_decodes_to_legacy_shape():
    frame = bytes([
        0, 0, 0, 21, 10, 19, 8, 1, 16, 100, 24, 5, 32, 2, 40, 101, 48, 7,
        56, 3, 64, 232, 7, 72, 42,
    ])
    assert md_wire.decode(frame) == {"BBO": [1, 100, 5, 2, 101, 7, 3, 1000, 42]}


def test_snapshot_decodes_levels():
    frame = bytes([
        0, 0, 0, 33, 18, 31, 8, 1, 18, 6, 8, 100, 16, 5, 24, 2, 18, 6, 8,
        99, 16, 3, 24, 1, 26, 6, 8, 101, 16, 7, 24, 3, 32, 208, 15, 40, 99,
    ])
    assert md_wire.decode(frame) == {
        "B": [1, [[100, 5, 2], [99, 3, 1]], [[101, 7, 3]], 2000, 99],
    }


def test_empty_snapshot_decodes():
    # The frame rsx-marketdata emits for a symbol with no book.
    frame = bytes([0, 0, 0, 4, 18, 2, 8, 1])
    assert md_wire.decode(frame) == {"B": [1, [], [], 0, 0]}


def test_delta_decodes():
    frame = bytes([
        0, 0, 0, 17, 26, 15, 8, 1, 16, 1, 24, 100, 32, 5, 40, 1, 48, 184,
        23, 56, 77,
    ])
    assert md_wire.decode(frame) == {"D": [1, 1, 100, 5, 1, 3000, 77]}


def test_trade_decodes_with_zero_taker_side():
    # taker_side = 0 is a proto3 default and is omitted on the wire; the
    # decoder must recover it as 0, not drop the field.
    frame = bytes([
        0, 0, 0, 14, 34, 12, 8, 4, 16, 172, 2, 24, 25, 40, 192, 62, 48, 66,
    ])
    assert md_wire.decode(frame) == {"T": [4, 300, 25, 0, 8000, 66]}


def test_heartbeat_decodes():
    frame = bytes([0, 0, 0, 5, 42, 3, 8, 185, 96])
    assert md_wire.decode(frame) == {"H": [12345]}


def test_heartbeat_zero_decodes():
    frame = bytes([0, 0, 0, 2, 42, 0])
    assert md_wire.decode(frame) == {"H": [0]}


def test_short_frame_returns_none():
    assert md_wire.decode(b"\x00\x00") is None
