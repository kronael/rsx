import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { useMarketStore } from "./store/market";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root element");

// Expose store for E2E benchmarks and Playwright tests.
// Safe: this is an internal dev/test UI, not a public API.
(window as unknown as Record<string, unknown>).__rsx = {
  applyL2Snapshot: useMarketStore.getState().applyL2Snapshot,
  applyL2Delta: useMarketStore.getState().applyL2Delta,
};

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
