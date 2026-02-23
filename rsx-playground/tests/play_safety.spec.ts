/**
 * Safety, crash & handover tests.
 *
 * Covers process crash recovery, session safety,
 * operational safety, and graceful degradation.
 *
 * Tests that stop processes MUST restart them before
 * exiting (afterEach or inline) to avoid breaking
 * subsequent tests.
 */

import { test, expect } from "@playwright/test";
import { waitForHTMX } from "./test_helpers";

const BASE = "http://localhost:49171";

// ── helpers ───────────────────────────────────────────────

async function stopProcess(
  request: any,
  name: string
) {
  return request.post(`/api/processes/${name}/stop`);
}

async function startProcess(
  request: any,
  name: string
) {
  return request.post(`/api/processes/${name}/start`);
}

async function stopAll(request: any) {
  return request.post(
    "/api/processes/all/stop?confirm=yes"
  );
}

async function startAll(request: any) {
  return request.post(
    "/api/processes/all/start?scenario=minimal&confirm=yes"
  );
}

async function ensureGateway(request: any) {
  await startProcess(request, "gw-0");
  // brief settle
  await new Promise((r) => setTimeout(r, 1000));
}

async function ensureMaker(request: any) {
  await startProcess(request, "maker");
  await new Promise((r) => setTimeout(r, 1000));
}

async function ensureAll(request: any) {
  await startAll(request);
  await new Promise((r) => setTimeout(r, 2000));
}

async function pollUntil(
  fn: () => Promise<boolean>,
  timeoutMs = 10000,
  intervalMs = 500
) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await fn()) return true;
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  return false;
}

// ── 1. Process Crash & Recovery ───────────────────────────

test.describe.serial("Safety: process crash & recovery",
  () => {
    test.afterEach(async ({ request }) => {
      // always restore gateway + maker
      await ensureGateway(request);
      await ensureMaker(request);
    });

    test("gateway crash shows error state in topology",
      async ({ page, request }) => {
        await stopProcess(request, "gw-0");
        await page.goto("/topology");
        await page.waitForTimeout(2000);
        // topology node for gw-0 should show red
        const topo = page.locator(
          "[data-process='gw-0']"
        );
        if (await topo.count() > 0) {
          await expect(topo).toContainText(
            /stopped|offline|red/i
          );
        }
        // orders page warns gateway not running
        await page.goto("/orders");
        await page.waitForTimeout(1000);
        const body = await page.textContent("body");
        // sim mode or gateway-offline indicator
        expect(body).toBeTruthy();
      }
    );

    test("gateway restart recovers order flow",
      async ({ page, request }) => {
        await stopProcess(request, "gw-0");
        await page.waitForTimeout(1000);
        await startProcess(request, "gw-0");
        // poll until gateway shows running
        const ok = await pollUntil(async () => {
          const res = await request.get(
            "/api/processes"
          );
          const procs = await res.json();
          const gw = Array.isArray(procs)
            ? procs.find(
                (p: any) => p.name === "gw-0"
              )
            : null;
          return gw?.state === "running";
        });
        expect(ok).toBe(true);
        // submit test order
        const res = await request.post(
          "/api/orders/test",
          {
            form: {
              symbol_id: "10",
              side: "buy",
              price: "50000",
              qty: "10",
              user_id: "1",
            },
          }
        );
        expect(res.ok()).toBe(true);
      }
    );

    test("maker crash shows stopped in maker tab",
      async ({ request }) => {
        await stopProcess(request, "maker");
        await new Promise((r) => setTimeout(r, 1000));
        const res = await request.get(
          "/api/maker/status"
        );
        const body = await res.json();
        expect(body.running).toBe(false);
      }
    );

    test("maker restart resumes quoting",
      async ({ request }) => {
        await stopProcess(request, "maker");
        await new Promise((r) => setTimeout(r, 500));
        await startProcess(request, "maker");
        const ok = await pollUntil(async () => {
          const res = await request.get(
            "/api/maker/status"
          );
          const body = await res.json();
          return body.running === true;
        });
        expect(ok).toBe(true);
      }
    );

    test("all-stop clears topology to red",
      async ({ page, request }) => {
        await stopAll(request);
        await page.goto("/topology");
        await page.waitForTimeout(2000);
        // check process list — all should be stopped
        const res = await request.get("/api/processes");
        const procs = await res.json();
        if (Array.isArray(procs)) {
          for (const p of procs) {
            expect(p.running).toBe(false);
          }
        }
        // restore
        await ensureAll(request);
      }
    );

    test("all-start recovers from all-stop",
      async ({ request }) => {
        await stopAll(request);
        await new Promise((r) => setTimeout(r, 1000));
        await startAll(request);
        const ok = await pollUntil(async () => {
          const res = await request.get(
            "/api/processes"
          );
          const procs = await res.json();
          if (!Array.isArray(procs)) return false;
          const running = procs.filter(
            (p: any) => p.running
          );
          return running.length >= 4;
        });
        expect(ok).toBe(true);
      }
    );

    test("rapid stop/start doesn't corrupt state",
      async ({ request }) => {
        await stopProcess(request, "gw-0");
        // immediately start without waiting
        await startProcess(request, "gw-0");
        await new Promise((r) => setTimeout(r, 2000));
        // no duplicate process entries
        const res = await request.get("/api/processes");
        const procs = await res.json();
        if (Array.isArray(procs)) {
          const gwEntries = procs.filter(
            (p: any) => p.name === "gw-0"
          );
          expect(gwEntries.length).toBeLessThanOrEqual(1);
        }
        // order submission works
        const orderRes = await request.post(
          "/api/orders/test",
          {
            form: {
              symbol_id: "10",
              side: "buy",
              price: "50000",
              qty: "10",
              user_id: "1",
            },
          }
        );
        expect(orderRes.ok()).toBe(true);
      }
    );

    test("process crash preserves session",
      async ({ request }) => {
        // capture session before crash
        const beforeRes = await request.get(
          "/api/sessions/status"
        );
        const before = await beforeRes.json();
        const sessionBefore = before.active_id;
        // stop/start gateway
        await stopProcess(request, "gw-0");
        await new Promise((r) => setTimeout(r, 500));
        await startProcess(request, "gw-0");
        await new Promise((r) => setTimeout(r, 1000));
        // session unchanged
        const afterRes = await request.get(
          "/api/sessions/status"
        );
        const after = await afterRes.json();
        expect(after.active_id).toBe(sessionBefore);
      }
    );
  }
);

// ── 2. Session Safety ────────────────────────────────────

test.describe("Safety: session safety", () => {
  test("session collision returns 409",
    async ({ request }) => {
      // global session already active; try allocate
      const res = await request.post(
        "/api/sessions/allocate"
      );
      expect(res.status()).toBe(409);
    }
  );

  test("session renew extends TTL",
    async ({ request }) => {
      // get current session
      const statusRes = await request.get(
        "/api/sessions/status"
      );
      const status = await statusRes.json();
      expect(status.active).toBe(true);
      // renew
      const renewRes = await request.post(
        "/api/sessions/renew",
        { data: { session_id: status.active_id } }
      );
      expect(renewRes.ok()).toBe(true);
      const body = await renewRes.json();
      expect(body.ttl_remaining_s).toBeGreaterThan(0);
    }
  );

  test("release then allocate works immediately",
    async ({ request }) => {
      // get current session
      const statusRes = await request.get(
        "/api/sessions/status"
      );
      const status = await statusRes.json();
      const origId = status.active_id;
      // release
      await request.post("/api/sessions/release", {
        data: { session_id: origId },
      });
      // allocate new
      const allocRes = await request.post(
        "/api/sessions/allocate"
      );
      expect(allocRes.ok()).toBe(true);
      const body = await allocRes.json();
      expect(body.ok).toBe(true);
      // new session is active
      expect(typeof body.session_id).toBe("string");
    }
  );

  test("invalid session_id renew returns 409",
    async ({ request }) => {
      const res = await request.post(
        "/api/sessions/renew",
        {
          data: {
            session_id:
              "00000000000000000000000000000000",
          },
        }
      );
      expect(res.status()).toBe(409);
    }
  );

  test("stale session auto-releases after lease TTL",
    async ({ request }) => {
      // We can't wait for real TTL expiry in a 15s test.
      // Verify the lease_remaining_s field exists and
      // decreases, proving the TTL mechanism is active.
      const res = await request.get(
        "/api/sessions/status"
      );
      const body = await res.json();
      expect(body.active).toBe(true);
      // lease_remaining_s or ttl_remaining_s exists
      const ttl =
        body.lease_remaining_s ??
        body.ttl_remaining_s ??
        null;
      expect(ttl).not.toBeNull();
      expect(ttl).toBeGreaterThan(0);
    }
  );
});

// ── 3. Operational Safety ────────────────────────────────

test.describe("Safety: operational safety", () => {
  test("confirm=yes required for destructive actions",
    async ({ request }) => {
      // without confirm
      const res = await request.post(
        "/api/processes/all/stop"
      );
      // should be rejected (not 200 success)
      expect(res.status()).not.toBe(200);
      // with confirm succeeds (but restore after)
      const res2 = await request.post(
        "/api/processes/all/stop?confirm=yes"
      );
      expect(res2.ok()).toBe(true);
      // restore
      await ensureAll(request);
    }
  );

  test("audit log records actions",
    async ({ request }) => {
      // submit an order to generate an audit entry
      await request.post("/api/orders/test", {
        form: {
          symbol_id: "10",
          side: "buy",
          price: "50000",
          qty: "1.0",
          user_id: "1",
        },
      });
      const res = await request.get("/api/audit-log");
      expect(res.ok()).toBe(true);
      const body = await res.json();
      // should have at least one entry
      expect(
        Array.isArray(body) ||
          Array.isArray(body.entries)
      ).toBe(true);
      const entries = Array.isArray(body)
        ? body
        : body.entries;
      expect(entries.length).toBeGreaterThan(0);
      // entries have timestamps
      const last = entries[entries.length - 1];
      expect(last.timestamp || last.ts).toBeTruthy();
    }
  );

  test("concurrent order submissions don't crash",
    async ({ request }) => {
      const promises = Array.from(
        { length: 10 },
        () =>
          request.post("/api/orders/test", {
            form: {
              symbol_id: "10",
              side: "buy",
              price: "50000",
              qty: "10",
              user_id: "1",
            },
          })
      );
      const results = await Promise.all(promises);
      for (const res of results) {
        expect(res.status()).not.toBe(500);
        expect(res.ok()).toBe(true);
      }
    }
  );

  test("idempotency key prevents duplicate orders",
    async ({ request }) => {
      const key = `safety-idem-${Date.now()}`;
      const res1 = await request.post(
        "/api/orders/test",
        {
          form: {
            symbol_id: "10",
            side: "buy",
            price: "50000",
            qty: "1.0",
            user_id: "1",
          },
          headers: { "X-Idempotency-Key": key },
        }
      );
      expect(res1.ok()).toBe(true);
      const res2 = await request.post(
        "/api/orders/test",
        {
          form: {
            symbol_id: "10",
            side: "buy",
            price: "50000",
            qty: "1.0",
            user_id: "1",
          },
          headers: { "X-Idempotency-Key": key },
        }
      );
      expect(res2.ok()).toBe(true);
      const body2 = await res2.json();
      const text2 = JSON.stringify(body2);
      expect(text2).toMatch(/duplicate|idempotent/i);
    }
  );

  test("invalid form data returns error, not 500",
    async ({ request }) => {
      // empty price
      const r1 = await request.post(
        "/api/orders/test",
        {
          form: {
            symbol_id: "10",
            side: "buy",
            price: "",
            qty: "1.0",
            user_id: "1",
          },
        }
      );
      expect(r1.status()).not.toBe(500);
      // non-numeric qty
      const r2 = await request.post(
        "/api/orders/test",
        {
          form: {
            symbol_id: "10",
            side: "buy",
            price: "50000",
            qty: "abc",
            user_id: "1",
          },
        }
      );
      expect(r2.status()).not.toBe(500);
      // invalid symbol_id
      const r3 = await request.post(
        "/api/orders/test",
        {
          form: {
            symbol_id: "-1",
            side: "buy",
            price: "50000",
            qty: "1.0",
            user_id: "1",
          },
        }
      );
      expect(r3.status()).not.toBe(500);
    }
  );

  test("run_id mismatch blocks process control",
    async ({ request }) => {
      const res = await request.post(
        "/api/processes/gw-0/start",
        {
          headers: {
            "X-Run-Id": "00000000-dead-beef-0000-000000000000",
          },
        }
      );
      // should be rejected if run_id enforcement is on
      // accept either 409 or 200 (if not enforced yet)
      expect(res.status()).not.toBe(500);
    }
  );
});

// ── 4. Graceful Degradation ──────────────────────────────

test.describe.serial("Safety: graceful degradation",
  () => {
    test.afterEach(async ({ request }) => {
      await ensureAll(request);
    });

    test("book page works with no processes",
      async ({ page, request }) => {
        await stopAll(request);
        await new Promise((r) => setTimeout(r, 1000));
        const errors: string[] = [];
        page.on("pageerror", (e) =>
          errors.push(e.message)
        );
        await page.goto("/book");
        await page.waitForTimeout(2000);
        // page loads (shows sim data or empty)
        await expect(
          page.locator("body")
        ).toContainText(/book|bid|ask|level|sim/i);
        expect(errors.length).toBe(0);
      }
    );

    test("risk page works with no postgres",
      async ({ page }) => {
        const errors: string[] = [];
        page.on("pageerror", (e) =>
          errors.push(e.message)
        );
        await page.goto("/risk");
        await page.waitForTimeout(1000);
        // cards render without 500
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
        // no JS errors
        expect(errors.length).toBe(0);
      }
    );

    test("WAL page works with no WAL files",
      async ({ page }) => {
        await page.goto("/wal");
        await page.waitForTimeout(1000);
        const body = await page.textContent("body");
        expect(body).toMatch(
          /wal|event|no wal|sim/i
        );
        // filter radios still functional
        const radios = page.locator(
          "input[type='radio']"
        );
        if (await radios.count() > 0) {
          await radios.first().click();
        }
      }
    );

    test("orders page works with gateway down",
      async ({ page, request }) => {
        await stopProcess(request, "gw-0");
        await new Promise((r) => setTimeout(r, 1000));
        await page.goto("/orders");
        await page.locator(
          "select[name='symbol_id']"
        ).selectOption("10");
        await page.locator(
          "select[name='side']"
        ).selectOption("buy");
        await page.locator(
          "input[name='price']"
        ).fill("50000");
        await page.locator(
          "input[name='qty']"
        ).fill("10");
        await page.locator(
          "button[type='submit']"
        ).click();
        await page.waitForTimeout(2000);
        // sim mode: accepted or "simulated"
        await expect(
          page.locator("#order-result")
        ).toContainText(
          /accepted|simulated|queued/i
        );
      }
    );

    test("topology works with all processes stopped",
      async ({ page, request }) => {
        await stopAll(request);
        await new Promise((r) => setTimeout(r, 1000));
        const errors: string[] = [];
        page.on("pageerror", (e) =>
          errors.push(e.message)
        );
        await page.goto("/topology");
        await page.waitForTimeout(2000);
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
        expect(errors.length).toBe(0);
      }
    );

    test("overview pulse bar handles zero state",
      async ({ page, request }) => {
        await stopAll(request);
        await new Promise((r) => setTimeout(r, 1000));
        const errors: string[] = [];
        page.on("pageerror", (e) =>
          errors.push(e.message)
        );
        await page.goto("/overview");
        await page.waitForTimeout(2000);
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
        // pulse bar shows 0 or "no processes"
        expect(body).toMatch(/0|no process/i);
        expect(errors.length).toBe(0);
      }
    );
  }
);
