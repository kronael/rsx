import clsx from "clsx";
import { useToastStore } from "../lib/toast";

const colors: Record<string, string> = {
  error: "bg-sell/90 text-white",
  success: "bg-buy/90 text-white",
  info: "bg-accent/90 text-white",
};

export function Toasts() {
  const toasts = useToastStore((s) => s.toasts);
  const remove = useToastStore((s) => s.remove);

  if (toasts.length === 0) return null;

  return (
    <div className="fixed bottom-4 right-4 z-50
      flex flex-col gap-2 max-w-xs"
    >
      {toasts.map((t) => (
        <div
          key={t.id}
          className={clsx(
            "px-3 py-2 rounded text-sm shadow-lg",
            "flex items-center justify-between gap-2",
            colors[t.type] ?? colors.info,
          )}
        >
          <span>{t.msg}</span>
          <button
            className="text-white/70 hover:text-white
              text-xs ml-2"
            onClick={() => remove(t.id)}
          >
            x
          </button>
        </div>
      ))}
    </div>
  );
}
