import { create } from "zustand";
import { WsStatus } from "../lib/types";

interface ConnectionStore {
  privateStatus: WsStatus;
  publicStatus: WsStatus;
  latency: number;

  setPrivateStatus: (s: WsStatus) => void;
  setPublicStatus: (s: WsStatus) => void;
  setLatency: (ms: number) => void;
}

export const useConnectionStore = create<ConnectionStore>(
  (set) => ({
    privateStatus: WsStatus.DISCONNECTED,
    publicStatus: WsStatus.DISCONNECTED,
    latency: 0,

    setPrivateStatus: (s) => set({ privateStatus: s }),
    setPublicStatus: (s) => set({ publicStatus: s }),
    setLatency: (ms) => set({ latency: ms }),
  }),
);
