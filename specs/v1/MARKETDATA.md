# Market Data Service (gRPC)

Market data is served by a dedicated service. It consumes orderbook events and exposes a minimal gRPC stream.

## Service

```proto
service MarketData {
    rpc Stream(MarketDataSubscribe) returns (stream MarketDataMessage);
}
```

## Subscribe

```proto
message MarketDataSubscribe {
    repeated uint32 symbol_id = 1;
    uint32 depth = 2;          // L2 depth per side (e.g., 10, 25, 50)
    bool send_snapshot = 3;    // If true, send L2Snapshot first
}
```

## Messages

```proto
message MarketDataMessage {
    oneof msg {
        BboUpdate bbo = 1;
        L2Snapshot snapshot = 2;
        L2Delta delta = 3;
    }
}

message BboUpdate {
    uint32 symbol_id = 1;
    int64 bid_px = 2;
    int64 bid_qty = 3;
    uint32 bid_count = 4;
    int64 ask_px = 5;
    int64 ask_qty = 6;
    uint32 ask_count = 7;
    uint64 timestamp_ns = 8;
    uint64 seq = 9;
}

message L2Snapshot {
    uint32 symbol_id = 1;
    repeated L2Level bids = 2;
    repeated L2Level asks = 3;
    uint64 timestamp_ns = 4;
    uint64 seq = 5;
}

message L2Delta {
    uint32 symbol_id = 1;
    Side side = 2;            // BUY=bid, SELL=ask
    int64 px = 3;
    int64 qty = 4;
    uint32 count = 5;
    uint64 timestamp_ns = 6;
    uint64 seq = 7;
}

message L2Level {
    int64 px = 1;
    int64 qty = 2;
    uint32 count = 3;
}
```

## Notes

- This mirrors the WS JSON schema (`B` snapshot, `D` delta) in gRPC form.
- BBO includes both price and quantity and order count.
- The event stream already exists in the matching engine; this service is a fan-out layer.

## Transport Details (v1)

- Public endpoint (no auth).
- Server-stream only; clients subscribe with `symbol_id` list.
- Backpressure: if client falls behind, server may **drop deltas**. Client must re-subscribe with `send_snapshot = true`.
- `seq` is the matching engine event height. Monotonic per symbol.
  Gap in seq -> client re-subscribes with `send_snapshot = true`.
- Server sends L2Snapshot on initial subscription before any deltas.
- Snapshot consistency: snapshot is point-in-time best effort, followed by deltas.
