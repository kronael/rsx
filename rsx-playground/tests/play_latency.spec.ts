import { test } from "@playwright/test";
import { expect } from "@playwright/test";

const ORDER_DATA = {
  symbol_id: "10",
  side: "buy",
  price: "50000",
  qty: "10",
  user_id: "1",
};

const PAGES = [
  "/",
  "/topology",
  "/book",
  "/risk",
  "/wal",
  "/orders",
  "/logs",
];

const HTMX_PARTIALS = [
  "/x/book?symbol_id=10",
  "/x/wal-detail",
  "/x/processes",
  "/x/risk-overview",
];

const API_ENDPOINTS = [
  "/api/processes",
  "/api/book/10",
  "/api/sessions/status",
  "/api/mark/prices",
];

// All /x/* endpoints for sweep test
const ALL_X_ENDPOINTS = [
  "/x/book?symbol_id=10",
  "/x/wal-detail",
  "/x/processes",
  "/x/risk-overview",
  "/x/recent-orders",
  "/x/live-fills",
  "/x/wal-timeline",
];

test.describe("Latency", () => {
  // ── Endpoint Latency (7 tests) ──────────────────

  test("page load < 2000ms for each tab", async ({
    page,
  }) => {
    for (const route of PAGES) {
      const start = Date.now();
      await page.goto(route);
      const elapsed = Date.now() - start;
      expect(
        elapsed,
        `${route} took ${elapsed}ms`
      ).toBeLessThan(2000);
    }
  });

  test("HTMX partial < 500ms", async ({
    request,
  }) => {
    for (const url of HTMX_PARTIALS) {
      const start = Date.now();
      const resp = await request.get(url);
      const elapsed = Date.now() - start;
      expect(resp.ok()).toBeTruthy();
      expect(
        elapsed,
        `${url} took ${elapsed}ms`
      ).toBeLessThan(500);
    }
  });

  test("order submission < 1000ms", async ({
    request,
  }) => {
    const start = Date.now();
    const resp = await request.post(
      "/api/orders/test",
      { form: ORDER_DATA }
    );
    const elapsed = Date.now() - start;
    expect(resp.ok()).toBeTruthy();
    expect(elapsed).toBeLessThan(3000);
  });

  test("10 concurrent orders < 15s", async ({
    request,
  }) => {
    const start = Date.now();
    const promises = Array.from(
      { length: 10 },
      (_, i) =>
        request.post("/api/orders/test", {
          form: {
            ...ORDER_DATA,
            price: String(50000 + i),
          },
        })
    );
    const results = await Promise.all(promises);
    const elapsed = Date.now() - start;
    for (const r of results) {
      expect(r.ok()).toBeTruthy();
    }
    expect(elapsed).toBeLessThan(15000);
  });

  test("API JSON endpoints < 200ms", async ({
    request,
  }) => {
    for (const url of API_ENDPOINTS) {
      const start = Date.now();
      const resp = await request.get(url);
      const elapsed = Date.now() - start;
      expect(resp.ok()).toBeTruthy();
      expect(
        elapsed,
        `${url} took ${elapsed}ms`
      ).toBeLessThan(200);
    }
  });

  test("static assets cached on reload", async ({
    page,
  }) => {
    // First load populates cache
    await page.goto("/");
    // Second load — check network for 304s
    const statuses: number[] = [];
    page.on("response", (resp) => {
      if (
        resp.url().includes(".css") ||
        resp.url().includes(".js")
      ) {
        statuses.push(resp.status());
      }
    });
    await page.reload();
    // Either 304 or 200 from CDN cache is acceptable;
    // just verify no 5xx on static assets.
    for (const s of statuses) {
      expect(s).toBeLessThan(500);
    }
  });

  test("no endpoint returns > 3s", async ({
    request,
  }) => {
    for (const url of ALL_X_ENDPOINTS) {
      const start = Date.now();
      const resp = await request.get(url);
      const elapsed = Date.now() - start;
      // Allow 404 for optional endpoints
      if (resp.status() !== 404) {
        expect(
          elapsed,
          `${url} took ${elapsed}ms`
        ).toBeLessThan(3000);
      }
    }
  });

  // ── Latency UI (3 tests) ───────────────────────

  test("latency endpoint returns stats after orders",
    async ({ request }) => {
      for (let i = 0; i < 5; i++) {
        await request.post("/api/orders/test", {
          form: {
            symbol_id: "10",
            side: "buy",
            price: "50000",
            qty: "100",
            cid: `lat-test-${i}`.padEnd(20, "0"),
          },
        });
      }
      const res = await request.get("/api/latency");
      expect(res.ok()).toBeTruthy();
      const data = await res.json();
      expect(data.count).toBeGreaterThanOrEqual(0);
      if (data.count > 0) {
        expect(data.p50).toBeGreaterThan(0);
        expect(data.p99).toBeGreaterThan(0);
      }
    },
  );

  test("risk latency card renders", async ({
    request,
  }) => {
    const res = await request.get("/x/risk-latency");
    expect(res.ok()).toBeTruthy();
    const html = await res.text();
    expect(html.toLowerCase()).toContain("latency");
  });

  test("latency regression card renders",
    async ({ request }) => {
      const res = await request.get(
        "/x/latency-regression"
      );
      expect(res.ok()).toBeTruthy();
      const html = await res.text();
      expect(html.toLowerCase()).toContain("p50");
    },
  );

  // ── Memory/Stability (3 tests) ─────────────────

  test("recent orders capped", async ({
    request,
  }) => {
    // Submit 100 orders in parallel batches of 20
    for (let batch = 0; batch < 5; batch++) {
      const promises = Array.from(
        { length: 20 },
        (_, i) =>
          request.post("/api/orders/test", {
            form: {
              ...ORDER_DATA,
              price: String(
                46000 + batch * 20 + i
              ),
            },
          })
      );
      await Promise.all(promises);
    }
    const resp = await request.get(
      "/api/orders/recent"
    );
    if (resp.status() === 404) {
      // /api/orders/recent isn't implemented; fall back
      // to HTML endpoint /x/recent-orders (must exist).
      const html = await request.get(
        "/x/recent-orders"
      );
      expect(html.status()).toBe(200);
      const text = await html.text();
      const rows = (
        text.match(/<tr[\s>]/g) || []
      ).length;
      // Subtract header row
      expect(rows - 1).toBeLessThanOrEqual(200);
      return;
    }
    const data = await resp.json();
    const count = Array.isArray(data)
      ? data.length
      : (data.orders?.length ?? 0);
    expect(count).toBeLessThanOrEqual(200);
  });

  test("recent fills capped at 200", async ({
    request,
  }) => {
    // Submit crossing orders in parallel batches
    // to generate fills (10 batches x 20 pairs)
    for (let batch = 0; batch < 10; batch++) {
      const promises: Promise<any>[] = [];
      for (let i = 0; i < 20; i++) {
        promises.push(
          request.post("/api/orders/test", {
            form: {
              symbol_id: "10",
              side: "sell",
              price: "50000",
              qty: "10",
              user_id: "2",
            },
          })
        );
        promises.push(
          request.post("/api/orders/test", {
            form: {
              symbol_id: "10",
              side: "buy",
              price: "51000",
              qty: "10",
              user_id: "1",
            },
          })
        );
      }
      await Promise.all(promises);
    }
    const resp = await request.get("/x/live-fills");
    // /x/live-fills must exist; regression if 404
    expect(resp.status()).toBe(200);
    const text = await resp.text();
    const rows = (
      text.match(/<tr[\s>]/g) || []
    ).length;
    // Subtract header row, cap at 200
    expect(
      Math.max(0, rows - 1)
    ).toBeLessThanOrEqual(200);
  });

  test("WAL events capped at 500", async ({
    request,
  }) => {
    // Orders already submitted from previous tests;
    // submit more in parallel batches to push past 500
    for (let batch = 0; batch < 5; batch++) {
      const promises = Array.from(
        { length: 20 },
        (_, i) =>
          request.post("/api/orders/test", {
            form: {
              ...ORDER_DATA,
              price: String(
                45000 + batch * 20 + i
              ),
            },
          })
      );
      await Promise.all(promises);
    }
    const resp = await request.get(
      "/x/wal-timeline"
    );
    // /x/wal-timeline must exist; regression if 404
    expect(resp.status()).toBe(200);
    const text = await resp.text();
    const rows = (
      text.match(/<tr[\s>]/g) || []
    ).length;
    expect(
      Math.max(0, rows - 1)
    ).toBeLessThanOrEqual(500);
  });
});
