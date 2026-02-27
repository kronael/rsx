import { create } from "zustand";
import type {
  UserPosition,
  UserOrder,
  UserFill,
} from "../lib/types";
import { OrderStatus } from "../lib/protocol";

interface AccountState {
  collateral: number;
  equity: number;
  pnl: number;
  im: number;
  mm: number;
  available: number;
}

interface TradingStore {
  positions: UserPosition[];
  positionsLoaded: boolean;
  orders: UserOrder[];
  fills: UserFill[];
  account: AccountState;

  setPositions: (p: UserPosition[]) => void;
  setOrders: (o: UserOrder[]) => void;
  setAccount: (a: AccountState) => void;
  updateOrder: (
    oid: string,
    status: OrderStatus,
    filled: number,
    remaining: number,
  ) => void;
  removeDoneOrder: (oid: string) => void;
  addFill: (f: UserFill) => void;
}

const emptyAccount: AccountState = {
  collateral: 0,
  equity: 0,
  pnl: 0,
  im: 0,
  mm: 0,
  available: 0,
};

export const useTradingStore = create<TradingStore>(
  (set) => ({
    positions: [],
    positionsLoaded: false,
    orders: [],
    fills: [],
    account: emptyAccount,

    setPositions: (p) => set({
      positions: p,
      positionsLoaded: true,
    }),
    setOrders: (o) => set({ orders: o }),
    setAccount: (a) => set({ account: a }),

    updateOrder: (oid, status, filled, _remaining) =>
      set((state) => {
        const done =
          status === OrderStatus.FILLED ||
          status === OrderStatus.CANCELLED ||
          status === OrderStatus.FAILED;
        if (done) {
          return {
            orders: state.orders.filter(
              (o) => o.oid !== oid,
            ),
          };
        }
        return {
          orders: state.orders.map((o) =>
            o.oid === oid
              ? { ...o, status, filled }
              : o,
          ),
        };
      }),

    removeDoneOrder: (oid) =>
      set((state) => ({
        orders: state.orders.filter((o) => o.oid !== oid),
      })),

    addFill: (f) =>
      set((state) => ({
        fills: [f, ...state.fills].slice(0, 200),
      })),
  }),
);
