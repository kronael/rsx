import { Side, OrderStatus, TIF } from "./protocol";

// Symbol metadata from M query
export interface SymbolMeta {
  id: number;
  name: string;
  tickSize: number;  // human float, e.g. 0.01
  lotSize: number;   // human float, e.g. 0.001
}

// Single orderbook price level
export interface PriceLevel {
  price: number;     // raw tick units
  qty: number;       // raw lot units
  count: number;     // order count
  total: number;     // cumulative qty
}

// Orderbook snapshot
export interface OrderbookState {
  bids: PriceLevel[];
  asks: PriceLevel[];
  spread: number;    // raw tick units
  spreadPct: number; // percentage
  midPrice: number;  // raw tick units
  seq: number;
}

// BBO state
export interface BboState {
  bidPx: number;
  bidQty: number;
  askPx: number;
  askQty: number;
  ts: number;
  seq: number;
}

// Recent trade
export interface RecentTrade {
  price: number;     // raw tick units
  qty: number;       // raw lot units
  side: Side;
  ts: number;        // nanoseconds
  seq: number;
}

// User order
export interface UserOrder {
  oid: string;
  cid: string;
  symbolId: number;
  side: Side;
  price: number;     // raw tick units
  qty: number;       // raw lot units
  filled: number;    // raw lot units
  status: OrderStatus;
  tif: TIF;
  reduceOnly: boolean;
  postOnly: boolean;
  ts: number;
}

// User position
export interface UserPosition {
  symbolId: number;
  side: Side;
  qty: number;       // raw lot units
  entryPx: number;   // raw tick units
  markPx: number;    // raw tick units
  unrealizedPnl: number; // raw tick units
  liqPx: number;     // raw tick units
}

// User fill
export interface UserFill {
  takerOid: string;
  makerOid: string;
  price: number;
  qty: number;
  ts: number;
  fee: number;
}

// 24h rolling statistics
export interface Stats24h {
  open: number;      // raw tick units
  high: number;      // raw tick units
  low: number;       // raw tick units
  close: number;     // raw tick units (last price)
  volume: number;    // raw lot units
  turnover: number;  // raw tick units (quote volume)
}

// Connection state
export const enum WsStatus {
  DISCONNECTED = "disconnected",
  CONNECTING = "connecting",
  CONNECTED = "connected",
  RECONNECTING = "reconnecting",
  OFFLINE = "offline",
  ERROR = "error",
}
