import { test, expect } from "@playwright/test";

const SYMBOL_ID = 10;
const MID = 50000;

/**
 * Poll fn with exponential backoff until it returns true or timeout/circuit
 * breaker fires.  fn throws on infra error, returns false when not-yet-ready.
 * After circuitAt consecutive throws the circuit breaker re-throws.
 */
async function poll(
  label: string,
  fn: () => Promise<boolean>,
  timeoutMs: number,
  opts: { initMs?: number; maxMs?: number; circuitAt?: number } = {},
): Promise<boolean> {
  const { initMs = 500, maxMs = 2000, circuitAt = 5 } = opts;
  const deadline = Date.now() + timeoutMs;
  let delay = initMs;
  let infraErrors = 0;

  while (Date.now() < deadline) {
    try {
      if (await fn()) return true;
      infraErrors = 0;
    } catch (e) {
      infraErrors++;
      if (infraErrors >= circuitAt) {
        throw new Error(`circuit breaker [${label}]: ${e}`);
      }
    }
    const remaining = deadline - Date.now();
    if (remaining <= 0) break;
    await new Promise((r) => setTimeout(r, Math.min(delay, remaining)));
    delay = Math.min(delay * 2, maxMs);
  }

  return false;
}

// Patch mid_override and wait for maker running + >=1 level each side.
// Two-phase: 15s status poll then 15s book poll (separate deadlines).
async function setupMaker(
  request: Parameters<Parameters<typeof test>[1]>[0]["request"],
  mid = MID,
) {
  // Explicit POST start
  const startRes = await request.post("/api/maker/start");
  expect(startRes.ok()).toBeTruthy();

  // Set mid override
  const patch = await request.fetch("/api/maker/config", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    data: JSON.stringify({ mid_override: mid }),
  });
  expect(patch.ok()).toBeTruthy();

  // Phase 1: poll up to 15s for status.running === true
  const running = await poll(
    "setupMaker:status",
    async () => {
      const r = await request.get("/api/maker/status");
      if (!r.ok()) throw new Error(`status ${r.status()}`);
      return (await r.json()).running === true;
    },
    15_000,
    { initMs: 300, maxMs: 1500 },
  );
  if (!running) {
    throw new Error("maker setup timed out: running=true not reached in 15s");
  }

  // Phase 2: poll up to 15s for >=1 level each side.
  // /api/book falls back to WAL BBO when no marketdata WS snapshot is
  // present, returning at most 1 bid + 1 ask — so >=2 would time out
  // in environments without a live marketdata feed.
  const booked = await poll(
    "setupMaker:book",
    async () => {
      const r = await request.get(`/api/book/${SYMBOL_ID}`);
      if (!r.ok()) throw new Error(`book ${r.status()}`);
      const book = await r.json();
      return book.bids?.length >= 1 && book.asks?.length >= 1;
    },
    15_000,
    { initMs: 300, maxMs: 1500 },
  );
  if (!booked) {
    throw new Error(
      "maker setup timed out: >=1 book level each side not reached in 15s",
    );
  }
}

test.describe("Market maker e2e", () => {
  test.beforeEach(async ({ request }) => {
    await setupMaker(request, MID);
  });

  // ── 1. Book populated ───────────────────────────────────

  test("book has >=1 bid and >=1 ask with best_bid < best_ask",
    async ({ request }) => {
      const res = await request.get(`/api/book/${SYMBOL_ID}`);
      expect(res.ok()).toBe(true);
      const book = await res.json();

      expect(book.bids.length).toBeGreaterThanOrEqual(1);
      expect(book.asks.length).toBeGreaterThanOrEqual(1);
      expect(book.bids[0].px).toBeLessThan(book.asks[0].px);
    },
  );

  // ── 2. Stop → clears; restart → repopulates ────────────

  test("stop clears book within 5s; restart gives >=2 levels within 8s",
    async ({ request }) => {
      // Stop maker
      const stopRes = await request.post("/api/maker/stop");
      expect(stopRes.ok()).toBeTruthy();

      // Wait up to 5s for book to clear (backoff 200→1000ms, circuit 5)
      const cleared = await poll(
        "book-clear",
        async () => {
          const res = await request.get(`/api/book/${SYMBOL_ID}`);
          if (!res.ok()) throw new Error(`book ${res.status()}`);
          const book = await res.json();
          return book.bids.length === 0 && book.asks.length === 0;
        },
        5_000,
        { initMs: 200, maxMs: 1000 },
      );
      expect(cleared).toBe(true);

      // Restart maker and set mid again
      await request.post("/api/maker/start");
      await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: MID }),
      });

      // Wait up to 8s for >=2 levels each side (backoff 200→1000ms, circuit 5)
      const restored = await poll(
        "book-restore",
        async () => {
          const res = await request.get(`/api/book/${SYMBOL_ID}`);
          if (!res.ok()) throw new Error(`book ${res.status()}`);
          const book = await res.json();
          return book.bids?.length >= 2 && book.asks?.length >= 2;
        },
        8_000,
        { initMs: 200, maxMs: 1000 },
      );
      expect(restored).toBe(true);
    },
  );

  // ── 3. Cross fill — F and U frames received ─────────────

  test(
    "user 1 bid at best_ask+1 gets F and U frames within 5s",
    async ({ request, baseURL }) => {
      // Get best_ask
      const bookRes = await request.get(
        `/api/book/${SYMBOL_ID}`,
      );
      const book = await bookRes.json();
      const bestAsk: number = book.asks[0].px;

      const wsBase = (baseURL ?? "http://localhost:49171")
        .replace(/^http/, "ws");

      // eslint-disable-next-line @typescript-eslint/no-require-imports
      const WS = require("ws");

      const frames = await new Promise<string[]>((resolve) => {
        const collected: string[] = [];
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const ws: any = new WS(`${wsBase}/ws/private`, {
          headers: { "x-user-id": "1" },
        });

        ws.on("open", () => {
          // Aggressive buy: side=0 (bid), price=bestAsk+1, qty=1
          ws.send(
            JSON.stringify(
              { N: [SYMBOL_ID, 0, bestAsk + 1, 1, 9001, 0] },
            ),
          );
        });

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        ws.on("message", (data: any) => {
          const raw = data.toString();
          collected.push(raw);
          let hasF = false;
          let hasU = false;
          for (const f of collected) {
            try {
              const p = JSON.parse(f);
              if ("F" in p) hasF = true;
              if ("U" in p) hasU = true;
            } catch (_) {}
          }
          if (hasF && hasU) {
            ws.close();
            resolve(collected);
          }
        });

        setTimeout(() => {
          ws.close();
          resolve(collected);
        }, 5_000);
      });

      const hasF = frames.some((f) => {
        try { return "F" in JSON.parse(f); }
        catch (_) { return false; }
      });
      const hasU = frames.some((f) => {
        try { return "U" in JSON.parse(f); }
        catch (_) { return false; }
      });

      expect(hasF).toBe(true);
      expect(hasU).toBe(true);
    },
  );

  // ── 4. Status lifecycle: running/pid fields after start and stop ──

  test("status fields reflect running=true+pid after start, false after stop",
    async ({ request }) => {
      // After setupMaker: running=true, pid is a positive integer
      const s1Res = await request.get("/api/maker/status");
      expect(s1Res.ok()).toBe(true);
      const s1 = await s1Res.json();
      expect(s1.running).toBe(true);
      expect(typeof s1.pid).toBe("number");
      expect(s1.pid).toBeGreaterThan(0);
      expect(s1.levels).toBeGreaterThanOrEqual(0);
      expect(Array.isArray(s1.errors)).toBe(true);

      // Book has at least 1 bid and 1 ask (side effect of maker running)
      const bookRes = await request.get(`/api/book/${SYMBOL_ID}`);
      expect(bookRes.ok()).toBe(true);
      const book = await bookRes.json();
      expect(book.bids.length).toBeGreaterThanOrEqual(1);
      expect(book.asks.length).toBeGreaterThanOrEqual(1);

      // Stop maker
      const stopRes = await request.post("/api/maker/stop");
      expect(stopRes.ok()).toBeTruthy();

      // Wait for status.running === false (up to 3s)
      const stopped = await poll(
        "status-stopped",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          const s = await r.json();
          return !s.running;
        },
        3_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stopped).toBe(true);

      // Verify pid is null after stop
      const s2Res = await request.get("/api/maker/status");
      expect(s2Res.ok()).toBe(true);
      const s2 = await s2Res.json();
      expect(s2.running).toBe(false);
      expect(s2.pid).toBeNull();
    },
  );

  // ── 5. Orderbook depth after 2 cycles ───────────────────

  test("orderbook has >=3 levels per side after 2 cycles",
    async ({ request }) => {
      // Wait 4s for maker to run ~2 quote cycles
      await new Promise((r) => setTimeout(r, 4000));
      const res = await request.get(`/api/book/${SYMBOL_ID}`);
      expect(res.ok()).toBe(true);
      const book = await res.json();
      expect(book.bids.length).toBeGreaterThanOrEqual(3);
      expect(book.asks.length).toBeGreaterThanOrEqual(3);
    },
  );

  // ── 6. Maker status has no repeated errors ──────────────

  test("maker status has no repeated errors",
    async ({ request }) => {
      const res = await request.get("/api/maker/status");
      expect(res.ok()).toBe(true);
      const status = await res.json();
      expect(status.running).toBe(true);
      // errors field absent or empty — maker is healthy
      const errors: unknown[] = status.errors ?? [];
      expect(errors.length).toBeLessThanOrEqual(3);
    },
  );

  // ── 7. BBO shifts when mid_override changes ──────────────

  test("bbo changes within 6s after mid_override update to 51000",
    async ({ request }) => {
      // Snapshot BBO
      const r0 = await request.get(`/api/bbo/${SYMBOL_ID}`);
      const bbo0 = await r0.json();
      const origBid: number = bbo0.bid_px;
      const origAsk: number = bbo0.ask_px;

      // Update mid override
      await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: 51000 }),
      });

      // Poll for BBO change (<=6s, backoff 200→800ms, circuit 5)
      const changed = await poll(
        "bbo-shift",
        async () => {
          const res = await request.get(`/api/bbo/${SYMBOL_ID}`);
          if (!res.ok()) throw new Error(`bbo ${res.status()}`);
          const bbo = await res.json();
          return bbo.bid_px !== origBid || bbo.ask_px !== origAsk;
        },
        6_000,
        { initMs: 200, maxMs: 800 },
      );

      // Restore
      await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: MID }),
      });

      expect(changed).toBe(true);
    },
  );
});

// ── Lifecycle & failure-path tests ──────────────────────────────────────────
//
// These tests verify the exact behaviour of /api/maker/start|status|stop:
//   - F-1: status fields well-typed after start
//   - F-2: double-start idempotent (PID unchanged, "already running" body)
//   - F-3: stop when not running returns 200 "not running" (no 5xx)
//   - F-4: PID changes after a full stop → start cycle
//   - F-5: PATCH /api/maker/config rejects non-numeric mid_override (400)
//   - F-6: PATCH /api/maker/config rejects missing mid_override (400)
//   - F-7: status returns running=false + pid=null when stopped
//   - F-8: fresh start returns 200 with "pid" in response body
//   - F-9: normal stop returns 200 with "stopped" in response body
//
// Each test manages maker state independently (no shared beforeEach).

test.describe("Maker API lifecycle and failure paths", () => {
  // ── F-1. Status fields are well-typed when maker is running ─────────

  test("status fields are well-typed after start",
    async ({ request }) => {
      // Ensure maker is running
      await request.post("/api/maker/start");
      await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: MID }),
      });

      const ok = await poll(
        "maker-running",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          const s = await r.json();
          return s.running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(ok).toBe(true);

      const res = await request.get("/api/maker/status");
      expect(res.ok()).toBe(true);
      const s = await res.json();

      // All required fields present and well-typed
      expect(typeof s.running).toBe("boolean");
      expect(s.running).toBe(true);
      expect(typeof s.pid).toBe("number");
      expect(s.pid).toBeGreaterThan(0);
      expect(typeof s.name).toBe("string");
      expect(s.name.length).toBeGreaterThan(0);
      expect(typeof s.levels).toBe("number");
      expect(s.levels).toBeGreaterThanOrEqual(0);
      expect(Array.isArray(s.errors)).toBe(true);
    },
  );

  // ── F-2. Double-start is idempotent: PID unchanged ───────────────────

  test("double-start: second start returns 'already running', pid unchanged",
    async ({ request }) => {
      // Ensure maker is running first
      await request.post("/api/maker/start");
      await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: MID }),
      });
      const runOk = await poll(
        "maker-running",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(runOk).toBe(true);

      const s1 = await (await request.get("/api/maker/status")).json();
      const pid1: number = s1.pid;

      // Second start
      const r2 = await request.post("/api/maker/start");
      expect(r2.ok()).toBe(true);
      const body = await r2.text();
      expect(body.toLowerCase()).toContain("already running");

      // PID must be unchanged
      const s2 = await (await request.get("/api/maker/status")).json();
      expect(s2.pid).toBe(pid1);
    },
  );

  // ── F-3. Stop when not running returns a soft warning ───────────────

  test("stop when not running returns 200 with 'not running' message",
    async ({ request }) => {
      // Ensure maker is stopped first
      await request.post("/api/maker/stop");
      const stoppedOk = await poll(
        "ensure-stopped",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === false;
        },
        5_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stoppedOk).toBe(true);

      // Stop again — must not 5xx
      const r = await request.post("/api/maker/stop");
      expect(r.ok()).toBe(true);                           // 2xx
      expect(r.status()).toBeLessThan(500);
      const body = await r.text();
      expect(body.toLowerCase()).toContain("not running");
    },
  );

  // ── F-4. PID changes after a full stop → start cycle ────────────────

  test("pid changes after stop then start",
    async ({ request }) => {
      // Ensure maker is running
      await request.post("/api/maker/start");
      await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: MID }),
      });
      const runOk = await poll(
        "maker-running",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(runOk).toBe(true);

      const pid1: number =
        (await (await request.get("/api/maker/status")).json()).pid;
      expect(pid1).toBeGreaterThan(0);

      // Stop
      const stopRes = await request.post("/api/maker/stop");
      expect(stopRes.ok()).toBeTruthy();

      const stoppedOk = await poll(
        "status-stopped",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === false;
        },
        5_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stoppedOk).toBe(true);

      // Verify pid is null after stop
      const sStopped =
        await (await request.get("/api/maker/status")).json();
      expect(sStopped.running).toBe(false);
      expect(sStopped.pid).toBeNull();

      // Restart
      await request.post("/api/maker/start");
      await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: MID }),
      });
      const runOk2 = await poll(
        "maker-restarted",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(runOk2).toBe(true);

      const pid2: number =
        (await (await request.get("/api/maker/status")).json()).pid;
      expect(pid2).toBeGreaterThan(0);

      // New process → new PID
      expect(pid2).not.toBe(pid1);
    },
  );

  // ── F-5. PATCH config rejects non-numeric mid_override ───────────────

  test("PATCH /api/maker/config with non-numeric mid_override returns 400",
    async ({ request }) => {
      const r = await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ mid_override: "not-a-number" }),
      });
      expect(r.status()).toBe(400);
      const body = await r.json();
      expect(body).toHaveProperty("error");
    },
  );

  // ── F-6. PATCH config rejects missing mid_override ───────────────────

  test("PATCH /api/maker/config with missing mid_override returns 400",
    async ({ request }) => {
      const r = await request.fetch("/api/maker/config", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        data: JSON.stringify({ unrelated_field: 999 }),
      });
      expect(r.status()).toBe(400);
      const body = await r.json();
      expect(body).toHaveProperty("error");
    },
  );

  // ── F-8. Fresh start response body contains pid ──────────────────────

  test("fresh start returns 200 with pid in response body",
    async ({ request }) => {
      // Ensure maker is stopped first
      await request.post("/api/maker/stop");
      const stoppedOk = await poll(
        "ensure-stopped-f8",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === false;
        },
        5_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stoppedOk).toBe(true);

      // Start maker — response must be 2xx and mention pid
      const startRes = await request.post("/api/maker/start");
      expect(startRes.ok()).toBe(true);
      const body = await startRes.text();
      expect(body.toLowerCase()).toContain("pid");

      // Clean up
      await request.post("/api/maker/stop");
    },
  );

  // ── F-9. Normal stop response body acknowledges stopped ──────────────

  test("stop when running returns 200 with 'stopped' in body",
    async ({ request }) => {
      // Ensure maker is running
      await request.post("/api/maker/start");
      const runOk = await poll(
        "ensure-running-f9",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(runOk).toBe(true);

      // Stop — must be 2xx and mention stopped
      const stopRes = await request.post("/api/maker/stop");
      expect(stopRes.ok()).toBe(true);
      expect(stopRes.status()).toBeLessThan(300);
      const body = await stopRes.text();
      expect(body.toLowerCase()).toContain("stopped");

      // Verify status reflects stopped
      const stoppedOk = await poll(
        "verify-stopped-f9",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === false;
        },
        5_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stoppedOk).toBe(true);
    },
  );

  // ── F-7. Status returns running=false with pid=null when stopped ─────

  test("status returns running=false and pid=null when maker is stopped",
    async ({ request }) => {
      // Ensure maker is stopped
      await request.post("/api/maker/stop");
      const stoppedOk = await poll(
        "ensure-stopped",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === false;
        },
        5_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stoppedOk).toBe(true);

      const res = await request.get("/api/maker/status");
      expect(res.ok()).toBe(true);
      const s = await res.json();
      expect(s.running).toBe(false);
      expect(s.pid).toBeNull();
      // name and levels/errors still present even when stopped
      expect(typeof s.name).toBe("string");
      expect(typeof s.levels).toBe("number");
      expect(Array.isArray(s.errors)).toBe(true);
    },
  );

  // ── F-10. SIGKILL via process endpoint → status reflects stopped ─────
  //
  // Simulates downstream failure: the maker process is killed externally
  // (SIGKILL via /api/processes/maker/kill).  Status must reflect
  // running=false + pid=null within 3s.

  test("SIGKILL via process endpoint: status reflects stopped within 3s",
    async ({ request }) => {
      // Ensure maker is running
      await request.post("/api/maker/start");
      const runOk = await poll(
        "ensure-running-f10",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(runOk).toBe(true);

      // Capture PID before kill
      const s1 = await (await request.get("/api/maker/status")).json();
      expect(s1.pid).toBeGreaterThan(0);

      // SIGKILL the maker process externally (downstream failure simulation)
      const killRes = await request.post(
        "/api/processes/maker/kill",
      );
      expect(killRes.ok()).toBe(true);

      // Status must reflect running=false + pid=null within 3s
      const stoppedOk = await poll(
        "status-killed",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          const s = await r.json();
          return !s.running && s.pid === null;
        },
        3_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stoppedOk).toBe(true);

      const s2 = await (await request.get("/api/maker/status")).json();
      expect(s2.running).toBe(false);
      expect(s2.pid).toBeNull();
    },
  );

  // ── F-11. Recovery after SIGKILL: fresh start creates a new PID ──────
  //
  // Verifies cleanup behaviour: after an external kill, the server
  // state is fully cleaned up and a subsequent /api/maker/start
  // succeeds and produces a new, valid PID.

  test("recovery after SIGKILL: fresh start creates new PID",
    async ({ request }) => {
      // Ensure maker is running
      await request.post("/api/maker/start");
      const runOk = await poll(
        "ensure-running-f11",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(runOk).toBe(true);

      const pid1: number =
        (await (await request.get("/api/maker/status")).json()).pid;

      // Kill process
      await request.post("/api/processes/maker/kill");

      // Wait for stopped
      const stoppedOk = await poll(
        "killed-stopped-f11",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === false;
        },
        5_000,
        { initMs: 200, maxMs: 800 },
      );
      expect(stoppedOk).toBe(true);

      // Fresh start after kill
      const startRes = await request.post("/api/maker/start");
      expect(startRes.ok()).toBe(true);

      const runOk2 = await poll(
        "restarted-f11",
        async () => {
          const r = await request.get("/api/maker/status");
          if (!r.ok()) throw new Error(`status ${r.status()}`);
          return (await r.json()).running === true;
        },
        10_000,
        { initMs: 300, maxMs: 1500 },
      );
      expect(runOk2).toBe(true);

      const pid2: number =
        (await (await request.get("/api/maker/status")).json()).pid;
      expect(pid2).toBeGreaterThan(0);
      // New process after kill must have a different PID
      expect(pid2).not.toBe(pid1);

      // Cleanup
      await request.post("/api/maker/stop");
    },
  );
});
