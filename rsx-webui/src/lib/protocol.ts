// RSX compact JSON wire protocol (WEBPROTO.md)
// Each frame is a JSON object with a single key.
// Key = message type, value = positional array.

export const enum Side {
  BUY = 0,
  SELL = 1,
}

export const enum TIF {
  GTC = 0,
  IOC = 1,
  FOK = 2,
}

export const enum OrderStatus {
  FILLED = 0,
  RESTING = 1,
  CANCELLED = 2,
  FAILED = 3,
}

export const enum FailureReason {
  INVALID_TICK_SIZE = 0,
  INVALID_LOT_SIZE = 1,
  SYMBOL_NOT_FOUND = 2,
  DUPLICATE_ORDER_ID = 3,
  INSUFFICIENT_MARGIN = 4,
  OVERLOADED = 5,
  INTERNAL_ERROR = 6,
  REDUCE_ONLY_VIOLATION = 7,
  POST_ONLY_REJECT = 8,
  RATE_LIMIT = 9,
  TIMEOUT = 10,
  USER_IN_LIQUIDATION = 11,
  WRONG_SHARD = 12,
}

// Channel bitmask for market data subscribe
export const enum MdChannel {
  BBO = 1,
  DEPTH = 2,
  TRADES = 4,
}

// --- Client -> Server (private WS) ---

export type NewOrderMsg = {
  N: [
    number, // sym
    Side,   // side
    number, // px (tick units)
    number, // qty (lot units)
    string, // cid (20 chars)
    TIF,    // tif
    number, // ro (0|1)
    number, // po (0|1)
  ];
};

export type CancelMsg = {
  C: [string]; // cid (20) or oid (32)
};

export type HeartbeatMsg = {
  H: [number]; // ts ms
};

// --- Client -> Server (public WS) ---

export type SubscribeMsg = {
  S: [number, number]; // sym, channels bitmask
};

export type UnsubscribeMsg = {
  X: [number, number]; // sym, channels bitmask
};

// --- Server -> Client (private WS) ---

export type OrderUpdateMsg = {
  U: [
    string,      // oid
    OrderStatus, // status
    number,      // filled qty
    number,      // remaining qty
    number,      // reason (FailureReason)
  ];
};

export type FillMsg = {
  F: [
    string, // taker_oid
    string, // maker_oid
    number, // px
    number, // qty
    number, // ts (ns)
    number, // fee
  ];
};

export type ErrorMsg = {
  E: [number, string]; // code, msg
};

// --- Server -> Client (public WS) ---

export type BboMsg = {
  BBO: [
    number, // sym
    number, // bid_px
    number, // bid_qty
    number, // bid_count
    number, // ask_px
    number, // ask_qty
    number, // ask_count
    number, // ts (ns)
    number, // seq
  ];
};

// L2 snapshot: bids [[px,qty,count],...],
// asks [[px,qty,count],...]
export type L2SnapshotMsg = {
  B: [
    number,       // sym
    number[][],   // bids
    number[][],   // asks
    number,       // ts
    number,       // seq
  ];
};

// L2 delta
export type L2DeltaMsg = {
  D: [
    number, // sym
    Side,   // side
    number, // px
    number, // qty
    number, // count
    number, // ts
    number, // seq
  ];
};

export type TradeMsg = {
  T: [
    number, // sym
    number, // px
    number, // qty
    Side,   // side
    number, // ts (ns)
    number, // seq
  ];
};

export type MetadataMsg = {
  M: [
    number, // sym
    string, // tick_size (human string)
    string, // lot_size (human string)
    string, // name
  ][];
};

// Union of all server messages
export type ServerMsg =
  | OrderUpdateMsg
  | FillMsg
  | ErrorMsg
  | HeartbeatMsg
  | BboMsg
  | L2SnapshotMsg
  | L2DeltaMsg
  | TradeMsg;

// Parse a raw JSON string into typed message.
// Returns null on unknown message type.
export function parseMessage(
  raw: string,
): ServerMsg | null {
  try {
    const msg = JSON.parse(raw) as Record<
      string,
      unknown
    >;
    const key = Object.keys(msg)[0];
    if (!key) return null;

    // Validate known message types
    const known = [
      "U", "F", "E", "H",
      "BBO", "B", "D", "T", "M",
    ];
    if (!known.includes(key)) return null;

    return msg as unknown as ServerMsg;
  } catch {
    return null;
  }
}

// Build a new order frame
export function newOrder(
  sym: number,
  side: Side,
  px: number,
  qty: number,
  cid: string,
  tif: TIF = TIF.GTC,
  reduceOnly = false,
  postOnly = false,
): string {
  const msg: NewOrderMsg = {
    N: [
      sym, side, px, qty,
      cid.padEnd(20, "0"),
      tif,
      reduceOnly ? 1 : 0,
      postOnly ? 1 : 0,
    ],
  };
  return JSON.stringify(msg);
}

// Build a cancel frame
export function cancelOrder(id: string): string {
  const msg: CancelMsg = { C: [id] };
  return JSON.stringify(msg);
}

// Build a heartbeat frame
export function heartbeat(): string {
  const msg: HeartbeatMsg = {
    H: [Date.now()],
  };
  return JSON.stringify(msg);
}

// Build a subscribe frame
export function subscribe(
  sym: number,
  channels: number,
): string {
  const msg: SubscribeMsg = {
    S: [sym, channels],
  };
  return JSON.stringify(msg);
}

// Build an unsubscribe frame
export function unsubscribe(
  sym: number,
  channels: number,
): string {
  const msg: UnsubscribeMsg = {
    X: [sym, channels],
  };
  return JSON.stringify(msg);
}
