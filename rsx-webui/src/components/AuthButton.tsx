import { useEffect, useState } from "react";
import clsx from "clsx";
import {
  clearToken,
  decodeClaims,
  getToken,
  isExpired,
  loginUrl,
  type AuthClaims,
} from "../lib/auth";

const AUTH_BASE =
  import.meta.env.VITE_AUTH_BASE || "/auth";

export function AuthButton() {
  const [claims, setClaims] = useState<AuthClaims | null>(
    null,
  );
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    const token = getToken();
    const c = decodeClaims(token);
    if (c && !isExpired(c)) {
      setClaims(c);
    } else if (c && isExpired(c)) {
      clearToken();
    }
  }, []);

  if (!claims) {
    return (
      <a
        href={loginUrl(AUTH_BASE)}
        className={clsx(
          "px-2 py-1 rounded text-xs font-medium",
          "bg-accent text-bg-base hover:opacity-90",
        )}
        data-testid="auth-login"
      >
        Sign in with GitHub
      </a>
    );
  }

  const label =
    claims.email || `user#${claims.user_id}`;

  return (
    <div className="relative" data-testid="auth-menu">
      <button
        type="button"
        onClick={() => setMenuOpen((v) => !v)}
        className={clsx(
          "px-2 py-1 rounded text-xs font-mono",
          "text-text-primary hover:bg-bg-surface",
        )}
        data-testid="auth-user"
      >
        {label}
      </button>
      {menuOpen && (
        <div
          className={clsx(
            "absolute right-0 top-full mt-1",
            "bg-bg-surface border border-border rounded",
            "shadow-lg z-50 min-w-[180px]",
          )}
        >
          <div className="px-3 py-2 text-xs text-text-secondary border-b border-border">
            user_id {claims.user_id}
          </div>
          <button
            type="button"
            onClick={() => {
              clearToken();
              setClaims(null);
              setMenuOpen(false);
            }}
            className={clsx(
              "w-full text-left px-3 py-2 text-xs",
              "hover:bg-bg-base",
            )}
            data-testid="auth-logout"
          >
            Sign out
          </button>
        </div>
      )}
    </div>
  );
}
