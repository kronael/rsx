import { create } from "zustand";

const STORAGE_KEY = "rsx_ui_settings";

interface SettingsStore {
  // Sound alerts on fill / liquidation
  soundEnabled: boolean;
  // Confirm modal before submitting market orders
  confirmMarketOrder: boolean;
  // Confirm modal before cancel-all
  confirmCancelAll: boolean;

  setSoundEnabled: (v: boolean) => void;
  setConfirmMarketOrder: (v: boolean) => void;
  setConfirmCancelAll: (v: boolean) => void;
}

function loadFromStorage(): Partial<SettingsStore> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    return JSON.parse(raw) as Partial<SettingsStore>;
  } catch {
    return {};
  }
}

function persist(state: Partial<SettingsStore>): void {
  try {
    const {
      soundEnabled,
      confirmMarketOrder,
      confirmCancelAll,
    } = state;
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        soundEnabled,
        confirmMarketOrder,
        confirmCancelAll,
      }),
    );
  } catch {
    // storage unavailable — ignore
  }
}

const saved = loadFromStorage();

export const useSettingsStore = create<SettingsStore>(
  (set) => ({
    soundEnabled: saved.soundEnabled ?? true,
    confirmMarketOrder: saved.confirmMarketOrder ?? true,
    confirmCancelAll: saved.confirmCancelAll ?? true,

    setSoundEnabled: (v) => {
      set({ soundEnabled: v });
      persist({ ...useSettingsStore.getState(), soundEnabled: v });
    },
    setConfirmMarketOrder: (v) => {
      set({ confirmMarketOrder: v });
      persist({
        ...useSettingsStore.getState(),
        confirmMarketOrder: v,
      });
    },
    setConfirmCancelAll: (v) => {
      set({ confirmCancelAll: v });
      persist({
        ...useSettingsStore.getState(),
        confirmCancelAll: v,
      });
    },
  }),
);
