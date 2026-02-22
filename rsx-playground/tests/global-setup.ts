/**
 * Global setup: start RSX processes before all tests.
 * Runs after playwright.config.ts webServer is ready (port 49171 up).
 */

const BASE = "http://localhost:49171";

async function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

/**
 * Poll fn until it returns true, timeout expires, or circuit breaker trips.
 *
 * fn must:
 *   - return true when the condition is satisfied
 *   - return false when not yet ready (transient; keep retrying)
 *   - throw on infrastructure-class failures (network down, crash)
 *
 * Retries use exponential backoff: initMs → 2x → … → maxMs.
 * After circuitAt consecutive infra throws the circuit breaker fires and
 * this function re-throws, failing the test run immediately.
 */
async function poll(
  label: string,
  fn: () => Promise<boolean>,
  timeoutMs: number,
  opts: { initMs?: number; maxMs?: number; circuitAt?: number } = {},
): Promise<boolean> {
  const { initMs = 500, maxMs = 4000, circuitAt = 5 } = opts;
  const deadline = Date.now() + timeoutMs;
  let delay = initMs;
  let infraErrors = 0;

  while (Date.now() < deadline) {
    try {
      if (await fn()) return true;
      infraErrors = 0; // successful response resets the counter
    } catch (e) {
      infraErrors++;
      const msg =
        `${label}: infra error ${infraErrors}/${circuitAt}: ${e}`;
      if (infraErrors >= circuitAt) {
        throw new Error(`global-setup circuit breaker: ${msg}`);
      }
      console.warn("global-setup:", msg);
    }

    const remaining = deadline - Date.now();
    if (remaining <= 0) break;
    await sleep(Math.min(delay, remaining));
    delay = Math.min(delay * 2, maxMs);
  }

  return false;
}

export default async function globalSetup() {
  console.log("global-setup: starting RSX processes...");

  // ── Session preflight: allocate exclusive run lock ──────
  // Retries with exponential backoff on transient infra errors.
  // Circuit breaker trips after COLLISION_MAX repeated 409s —
  // a single collision might be a ghost session about to expire;
  // repeated collisions mean a live conflicting run.
  const ALLOC_MAX = 5;      // max total attempts
  const COLLISION_MAX = 3;  // repeated 409s before hard-fail
  let sessionId: string | null = null;
  let runId: string | null = null;
  {
    let delay = 1000;
    let attempts = 0;
    let collisions = 0;
    while (attempts < ALLOC_MAX) {
      attempts++;
      let r: Response;
      try {
        r = await fetch(`${BASE}/api/sessions/allocate`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({}),
          signal: AbortSignal.timeout(5000),
        });
      } catch (e) {
        console.warn(
          `global-setup: session allocate attempt` +
            ` ${attempts}/${ALLOC_MAX} failed: ${e}`,
        );
        if (attempts < ALLOC_MAX) {
          await sleep(delay);
          delay = Math.min(delay * 2, 16000);
        }
        continue;
      }

      if (r.status === 409) {
        const body = await r.json().catch(() => ({}));
        collisions++;
        if (collisions >= COLLISION_MAX) {
          throw new Error(
            `global-setup: session collision circuit breaker` +
              ` after ${collisions} repeated 409s —` +
              ` another run is active.` +
              ` active_id=${body.active_id ?? "?"}` +
              ` age=${body.age_s ?? "?"}s.` +
              ` Abort or wait for the existing run to finish.`,
          );
        }
        console.warn(
          `global-setup: session collision` +
            ` ${collisions}/${COLLISION_MAX}` +
            ` (active_id=${body.active_id ?? "?"}` +
            ` age=${body.age_s ?? "?"}s);` +
            ` retrying in ${delay / 1000}s`,
        );
        await sleep(delay);
        delay = Math.min(delay * 2, 16000);
        continue;
      }

      if (!r.ok) {
        console.warn(
          "global-setup: session allocate failed (non-fatal):",
          r.status,
        );
        break;
      }

      const body = await r.json();
      sessionId = body.session_id ?? null;
      runId = body.run_id ?? null;
      const tag = body.reclaimed ? " (reclaimed)" : "";
      console.log(
        `global-setup: session allocated${tag}` +
          ` (session=${sessionId} run=${runId})`,
      );
      break;
    }
  }

  // ── Heartbeat: renew session every 2 min to stay within ──
  // the 5-min lease window and prevent stale-claim eviction.
  let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  if (sessionId) {
    heartbeatTimer = setInterval(async () => {
      try {
        await fetch(`${BASE}/api/sessions/renew`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ session_id: sessionId }),
          signal: AbortSignal.timeout(3000),
        });
      } catch (e) {
        console.warn("global-setup: heartbeat renew failed:", e);
      }
    }, 120_000); // every 2 min
  }

  // ── Teardown: release session lock when run completes ───
  const releaseFn = async () => {
    if (heartbeatTimer !== null) {
      clearInterval(heartbeatTimer);
      heartbeatTimer = null;
    }
    if (!sessionId) return;
    try {
      await fetch(`${BASE}/api/sessions/release`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_id: sessionId }),
        signal: AbortSignal.timeout(3000),
      });
      console.log(`global-setup: session released (${sessionId})`);
    } catch (e) {
      console.warn("global-setup: session release failed:", e);
    }
  };

  // Start all processes — response arrives after cargo build + spawn (up to 2m)
  // X-Run-Id authenticates this caller as the active session owner;
  // server hard-fails (409) if the run_id is stale or mismatched.
  try {
    const startHeaders: Record<string, string> = {};
    if (runId) startHeaders["X-Run-Id"] = runId;
    const res = await fetch(
      `${BASE}/api/processes/all/start?scenario=minimal&confirm=yes`,
      {
        method: "POST",
        headers: startHeaders,
        signal: AbortSignal.timeout(120_000),
      },
    );
    const html = await res.text();
    const text = html.replace(/<[^>]+>/g, "").trim();
    console.log("global-setup: start response:", text);
  } catch (e) {
    console.error("global-setup: start request failed:", e);
    await releaseFn();
    return;
  }

  // Poll /api/processes until gateway is running (up to 30s)
  const gatewayReady = await poll(
    "processes",
    async () => {
      const r = await fetch(`${BASE}/api/processes`, {
        signal: AbortSignal.timeout(5000),
      });
      if (!r.ok) return false;
      const procs: Array<{ name: string; state: string }> = await r.json();
      const running = procs.filter((p) => p.state === "running");
      const gatewayUp = running.some(
        (p) => p.name.includes("gateway") || p.name.startsWith("gw"),
      );
      console.log(
        `global-setup: ${running.length} processes running` +
          (gatewayUp ? " (gateway up)" : ""),
      );
      return gatewayUp && running.length >= 4;
    },
    30_000,
  );

  if (!gatewayReady) {
    console.warn("global-setup: warning: gateway not ready after 30s");
  }

  // Extra wait for market maker to seed the PENGU book
  await sleep(5000);

  // Poll /api/maker/status until running=true (15s timeout)
  const makerRunning = await poll(
    "maker",
    async () => {
      const r = await fetch(`${BASE}/api/maker/status`, {
        signal: AbortSignal.timeout(3000),
      });
      if (!r.ok) return false;
      const status = await r.json();
      return Boolean(status.running);
    },
    15_000,
    { initMs: 500, maxMs: 2000 },
  );

  if (!makerRunning) {
    console.warn("global-setup: warning: maker not running after 15s");
  }

  // Poll /api/book/10 until best_bid > 0 (8s timeout)
  const bookSeeded = await poll(
    "book",
    async () => {
      const r = await fetch(`${BASE}/api/book/10`, {
        signal: AbortSignal.timeout(3000),
      });
      if (!r.ok) return false;
      const book = await r.json();
      const bestBid = book.bids?.[0]?.px ?? 0;
      return bestBid > 0;
    },
    8_000,
    { initMs: 500, maxMs: 2000 },
  );

  if (!bookSeeded) {
    console.warn("global-setup: warning: book not seeded after 8s");
  }

  if (makerRunning && bookSeeded) {
    console.log("global-setup: maker running, book seeded");
  }

  console.log("global-setup: RSX processes ready");

  // Return teardown so Playwright releases the session lock
  // after all tests complete (success or failure).
  return releaseFn;
}
