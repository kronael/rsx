/**
 * Auth helpers — reads JWT from cookie or localStorage,
 * exposes claims for UI display.
 */

const TOKEN_COOKIE = "rsx_token";
const TOKEN_STORAGE = "rsx_token";

export interface AuthClaims {
  sub: string;
  user_id: number;
  email: string | null;
  iat: number;
  exp: number;
}

function readCookie(name: string): string | null {
  const match = document.cookie.match(
    new RegExp(`(?:^|;\\s*)${name}=([^;]+)`),
  );
  if (!match || match[1] === undefined) return null;
  return decodeURIComponent(match[1]);
}

export function getToken(): string | null {
  const fromCookie = readCookie(TOKEN_COOKIE);
  if (fromCookie) {
    // Mirror cookie to localStorage so SPA survives
    // without needing JS-readable cookies after first load
    try {
      localStorage.setItem(TOKEN_STORAGE, fromCookie);
    } catch {
      /* ignore */
    }
    return fromCookie;
  }
  try {
    return localStorage.getItem(TOKEN_STORAGE);
  } catch {
    return null;
  }
}

export function decodeClaims(
  token: string | null,
): AuthClaims | null {
  if (!token) return null;
  const parts = token.split(".");
  if (parts.length !== 3 || !parts[1]) return null;
  try {
    const payload = JSON.parse(
      atob(parts[1].replace(/-/g, "+").replace(/_/g, "/")),
    );
    if (
      typeof payload.user_id === "number" &&
      typeof payload.sub === "string"
    ) {
      return payload as AuthClaims;
    }
    return null;
  } catch {
    return null;
  }
}

export function isExpired(claims: AuthClaims | null): boolean {
  if (!claims) return true;
  return claims.exp * 1000 < Date.now();
}

export function clearToken(): void {
  try {
    localStorage.removeItem(TOKEN_STORAGE);
  } catch {
    /* ignore */
  }
  document.cookie =
    `${TOKEN_COOKIE}=; Max-Age=0; path=/`;
}

/**
 * Entry point: build GitHub OAuth login URL.
 * `authBase` is the rsx-auth service base URL (env/config).
 */
export function loginUrl(
  authBase: string,
  redirectBack: string = window.location.href,
): string {
  const q = new URLSearchParams({
    redirect: redirectBack,
  });
  return `${authBase}/oauth/github/login?${q.toString()}`;
}
