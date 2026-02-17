import { TradeLayout } from "./components/layout/TradeLayout";
import { TopBar } from "./components/layout/TopBar";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Toasts } from "./components/Toast";
import { useSoundAlerts } from "./hooks/useSoundAlerts";

function AppInner() {
  useSoundAlerts();
  return (
    <div className="flex flex-col h-screen
      bg-bg-primary text-text-primary font-sans">
      <TopBar />
      <TradeLayout />
    </div>
  );
}

export function App() {
  return (
    <ErrorBoundary>
      <AppInner />
      <Toasts />
    </ErrorBoundary>
  );
}
