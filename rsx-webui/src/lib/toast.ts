import { create } from "zustand";

interface Toast {
  id: number;
  msg: string;
  type: "error" | "success" | "info";
}

let nextId = 1;

interface ToastStore {
  toasts: Toast[];
  add: (msg: string, type: Toast["type"]) => void;
  remove: (id: number) => void;
}

export const useToastStore = create<ToastStore>(
  (set) => ({
    toasts: [],
    add: (msg, type) => {
      const id = nextId++;
      set((s) => ({
        toasts: [...s.toasts, { id, msg, type }],
      }));
      setTimeout(() => {
        set((s) => ({
          toasts: s.toasts.filter((t) => t.id !== id),
        }));
      }, 4000);
    },
    remove: (id) =>
      set((s) => ({
        toasts: s.toasts.filter((t) => t.id !== id),
      })),
  }),
);
