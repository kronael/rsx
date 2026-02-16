import { TradeLayout } from "./components/layout/TradeLayout";
import { TopBar } from "./components/layout/TopBar";

export function App() {
  return (
    <div className="flex flex-col h-screen
      bg-bg-primary text-text-primary font-sans">
      <TopBar />
      <TradeLayout />
    </div>
  );
}
