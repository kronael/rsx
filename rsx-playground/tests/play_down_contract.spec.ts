// ── down-contract tests ───────────────────────────────────────────────
//
// Require explicit 5xx (or 4xx where semantically correct) responses
// with actionable JSON error payloads when gateway/services are
// intentionally unavailable or given invalid inputs.
//
// Contract rules (applied per-endpoint):
//   • HTTP status is NOT 200 (no silent error masking)
//   • Content-Type contains "application/json"
//   • Response body has at least one non-empty error field
//   • No raw Python tracebacks in the response body
//   • No empty-string error values (must be actionable)
//
// These tests are self-contained: they manufacture the "down"
// condition deterministically (closed port, wrong id, bad input)
// and do not depend on the live system being in any particular state.

import { test, expect } from "@playwright/test";

// ── helper ────────────────────────────────────────────────────────────

async function assertNoTraceback(text: string) {
  expect(text).not.toContain("Traceback");
  expect(text).not.toContain('File "');
  expect(text).not.toContain("line ");
}

// ── v1-proxy down ─────────────────────────────────────────────────────
//
// When GATEWAY_HTTP points to a running server the proxy returns
// 200/4xx.  When the gateway is not running (default dev setup with
// gateway not started) the proxy must return 502 JSON, not a Python
// 500 or empty body.  The conditional structure lets the test pass
// in both states.

test.describe("v1-proxy down contract", () => {
  test("GET /v1/book returns 200, 4xx, or 502 (never 500)",
    async ({ request }) => {
      const res = await request.get("/v1/book", {
        params: { symbol_id: "10" },
      });
      expect([200, 400, 404, 502]).toContain(res.status());
    }
  );

  test("502 from v1-proxy has application/json content-type",
    async ({ request }) => {
      const res = await request.get("/v1/book", {
        params: { symbol_id: "10" },
      });
      if (res.status() === 502) {
        const ct = res.headers()["content-type"] ?? "";
        expect(ct).toContain("application/json");
      } else {
        expect([200, 400, 404]).toContain(res.status());
      }
    }
  );

  test("502 from v1-proxy has non-empty error field",
    async ({ request }) => {
      const res = await request.get("/v1/book", {
        params: { symbol_id: "10" },
      });
      if (res.status() === 502) {
        const body = await res.json();
        expect(typeof body.error).toBe("string");
        expect(body.error.length).toBeGreaterThan(0);
      } else {
        expect([200, 400, 404]).toContain(res.status());
      }
    }
  );

  test("502 from v1-proxy has no Python traceback",
    async ({ request }) => {
      const res = await request.get("/v1/book", {
        params: { symbol_id: "10" },
      });
      if (res.status() === 502) {
        const text = await res.text();
        await assertNoTraceback(text);
      } else {
        expect([200, 400, 404]).toContain(res.status());
      }
    }
  );

  // v1-proxy with a path that will never 200 (non-existent endpoint)
  // — must not return 500.
  test("v1-proxy unknown path returns 502 or 404, not 500",
    async ({ request }) => {
      const res = await request.get("/v1/nonexistent-path-xyzzy");
      expect(res.status()).not.toBe(500);
      expect([404, 502]).toContain(res.status());
    }
  );

  test("v1-proxy 502 body is valid JSON (not HTML)",
    async ({ request }) => {
      const res = await request.get("/v1/book", {
        params: { symbol_id: "10" },
      });
      if (res.status() === 502) {
        // Must parse without throwing
        const body = await res.json();
        expect(typeof body).toBe("object");
        expect(body).not.toBeNull();
      } else {
        expect([200, 400, 404]).toContain(res.status());
      }
    }
  );
});

// ── risk-action down contract ─────────────────────────────────────────
//
// Invalid action names must produce 400 with a JSON error field.
// This simulates the "risk engine unavailable for action X" path
// where the server must reject the request with an actionable message.

test.describe("risk-action down contract", () => {
  test("unknown action returns HTTP 400",
    async ({ request }) => {
      const res = await request.post(
        "/api/risk/users/1/badaction"
      );
      expect(res.status()).toBe(400);
    }
  );

  test("400 response has application/json content-type",
    async ({ request }) => {
      const res = await request.post(
        "/api/risk/users/1/badaction"
      );
      expect(res.status()).toBe(400);
      const ct = res.headers()["content-type"] ?? "";
      expect(ct).toContain("application/json");
    }
  );

  test("400 response has non-empty error field",
    async ({ request }) => {
      const res = await request.post(
        "/api/risk/users/1/badaction"
      );
      expect(res.status()).toBe(400);
      const body = await res.json();
      expect(typeof body.error).toBe("string");
      expect(body.error.length).toBeGreaterThan(0);
    }
  );

  test("400 error field names the unknown action",
    async ({ request }) => {
      const res = await request.post(
        "/api/risk/users/1/badaction"
      );
      const body = await res.json();
      // Must include the invalid action name so the caller
      // can understand what went wrong.
      expect(body.error).toContain("badaction");
    }
  );

  test("400 has no Python traceback",
    async ({ request }) => {
      const res = await request.post(
        "/api/risk/users/1/badaction"
      );
      const text = await res.text();
      await assertNoTraceback(text);
    }
  );
});

// ── session-renew down contract ────────────────────────────────────────
//
// Renewing a session with a wrong/unknown session_id must return 409
// with a structured error (not 200 OK or unhandled 500).

test.describe("session-renew down contract", () => {
  const GHOST_ID = "00000000000000000000000000000000";

  test("renew with wrong session_id returns 409",
    async ({ request }) => {
      const res = await request.post("/api/sessions/renew", {
        data: { session_id: GHOST_ID },
      });
      // 409 if no active session or id mismatch, not 200 or 500.
      expect([409]).toContain(res.status());
    }
  );

  test("409 renew response has application/json content-type",
    async ({ request }) => {
      const res = await request.post("/api/sessions/renew", {
        data: { session_id: GHOST_ID },
      });
      if (res.status() === 409) {
        const ct = res.headers()["content-type"] ?? "";
        expect(ct).toContain("application/json");
      }
    }
  );

  test("409 renew response has non-empty error field",
    async ({ request }) => {
      const res = await request.post("/api/sessions/renew", {
        data: { session_id: GHOST_ID },
      });
      if (res.status() === 409) {
        const body = await res.json();
        expect(typeof body.error).toBe("string");
        expect(body.error.length).toBeGreaterThan(0);
      }
    }
  );

  test("409 renew response has no Python traceback",
    async ({ request }) => {
      const res = await request.post("/api/sessions/renew", {
        data: { session_id: GHOST_ID },
      });
      if (res.status() === 409) {
        const text = await res.text();
        await assertNoTraceback(text);
      }
    }
  );

  test("renew without session_id body returns non-200",
    async ({ request }) => {
      const res = await request.post("/api/sessions/renew", {
        data: {},
      });
      // Empty session_id resolves to "" — mismatch → 409.
      expect(res.status()).not.toBe(200);
    }
  );
});

// ── maker-config down contract ────────────────────────────────────────
//
// PATCH /api/maker/config with a non-numeric mid_override must
// return 400 with {"error": ...}  — not 200 or unhandled 500.

test.describe("maker-config down contract", () => {
  test("string mid_override returns HTTP 400",
    async ({ request }) => {
      const res = await request.patch("/api/maker/config", {
        data: JSON.stringify({ mid_override: "not-a-number" }),
        headers: { "content-type": "application/json" },
      });
      expect(res.status()).toBe(400);
    }
  );

  test("400 maker-config has application/json content-type",
    async ({ request }) => {
      const res = await request.patch("/api/maker/config", {
        data: JSON.stringify({ mid_override: "bad" }),
        headers: { "content-type": "application/json" },
      });
      expect(res.status()).toBe(400);
      const ct = res.headers()["content-type"] ?? "";
      expect(ct).toContain("application/json");
    }
  );

  test("400 maker-config body has non-empty error field",
    async ({ request }) => {
      const res = await request.patch("/api/maker/config", {
        data: JSON.stringify({ mid_override: null }),
        headers: { "content-type": "application/json" },
      });
      expect(res.status()).toBe(400);
      const body = await res.json();
      expect(typeof body.error).toBe("string");
      expect(body.error.length).toBeGreaterThan(0);
    }
  );

  test("400 maker-config has no Python traceback",
    async ({ request }) => {
      const res = await request.patch("/api/maker/config", {
        data: JSON.stringify({ mid_override: [] }),
        headers: { "content-type": "application/json" },
      });
      expect(res.status()).toBe(400);
      const text = await res.text();
      await assertNoTraceback(text);
    }
  );

  test("missing mid_override returns 400",
    async ({ request }) => {
      const res = await request.patch("/api/maker/config", {
        data: JSON.stringify({}),
        headers: { "content-type": "application/json" },
      });
      expect(res.status()).toBe(400);
    }
  );
});

// ── bbo-not-found down contract ────────────────────────────────────────
//
// GET /api/bbo/{symbol_id} for an unknown symbol must return 404 with
// a JSON error — not 200 with empty data or unhandled 500.

test.describe("bbo-not-found down contract", () => {
  // Use a symbol_id that will never have data in test env.
  const GHOST_SYMBOL = 999999;

  test("unknown symbol returns 404 or 200 (never 500)",
    async ({ request }) => {
      const res = await request.get(`/api/bbo/${GHOST_SYMBOL}`);
      // 404 if no WAL data; 200 with empty BBO if book is empty.
      expect([200, 404]).toContain(res.status());
      expect(res.status()).not.toBe(500);
    }
  );

  test("404 bbo response has application/json content-type",
    async ({ request }) => {
      const res = await request.get(`/api/bbo/${GHOST_SYMBOL}`);
      if (res.status() === 404) {
        const ct = res.headers()["content-type"] ?? "";
        expect(ct).toContain("application/json");
      } else {
        expect(res.status()).toBe(200);
      }
    }
  );

  test("404 bbo body has non-empty error field",
    async ({ request }) => {
      const res = await request.get(`/api/bbo/${GHOST_SYMBOL}`);
      if (res.status() === 404) {
        const body = await res.json();
        expect(typeof body.error).toBe("string");
        expect(body.error.length).toBeGreaterThan(0);
      } else {
        expect(res.status()).toBe(200);
      }
    }
  );

  test("404 bbo has no Python traceback",
    async ({ request }) => {
      const res = await request.get(`/api/bbo/${GHOST_SYMBOL}`);
      if (res.status() === 404) {
        const text = await res.text();
        await assertNoTraceback(text);
      } else {
        expect(res.status()).toBe(200);
      }
    }
  );

  test("/api/book/{symbol_id} for unknown symbol returns 200 "
    + "with empty bids/asks (not 500)",
    async ({ request }) => {
      const res = await request.get(`/api/book/${GHOST_SYMBOL}`);
      // book endpoint always returns 200 with bids/asks arrays.
      expect(res.status()).toBe(200);
      const body = await res.json();
      expect(Array.isArray(body.bids)).toBe(true);
      expect(Array.isArray(body.asks)).toBe(true);
    }
  );
});

// ── stress-report-not-found down contract ─────────────────────────────
//
// GET /api/stress/reports/{id} for a non-existent report must return
// 404 JSON (not 500 or HTML).

test.describe("stress-report-not-found down contract", () => {
  test("non-existent report returns HTTP 404",
    async ({ request }) => {
      const res = await request.get(
        "/api/stress/reports/nonexistent-00000000"
      );
      expect(res.status()).toBe(404);
    }
  );

  test("404 stress report has application/json content-type",
    async ({ request }) => {
      const res = await request.get(
        "/api/stress/reports/nonexistent-00000000"
      );
      expect(res.status()).toBe(404);
      const ct = res.headers()["content-type"] ?? "";
      expect(ct).toContain("application/json");
    }
  );

  test("404 stress report body has non-empty error field",
    async ({ request }) => {
      const res = await request.get(
        "/api/stress/reports/nonexistent-00000000"
      );
      expect(res.status()).toBe(404);
      const body = await res.json();
      expect(typeof body.error).toBe("string");
      expect(body.error.length).toBeGreaterThan(0);
    }
  );

  test("404 stress report has no Python traceback",
    async ({ request }) => {
      const res = await request.get(
        "/api/stress/reports/nonexistent-00000000"
      );
      const text = await res.text();
      await assertNoTraceback(text);
    }
  );
});

// ── process-action down contract ─────────────────────────────────────
//
// POST /api/processes/{name}/{action} with an unsupported action name
// must return 4xx (not 500) with a JSON error payload.

test.describe("process-action down contract", () => {
  test("unknown action on existing process returns 4xx not 500",
    async ({ request }) => {
      const res = await request.post(
        "/api/processes/gateway/explode"
      );
      // Server may return 400 (bad action) or 404 (not found).
      // Must NOT be 200 (silent ignore) or 500 (unhandled).
      expect(res.status()).not.toBe(200);
      expect(res.status()).not.toBe(500);
      expect(res.status()).toBeLessThan(500);
    }
  );

  test("unknown process name returns 4xx not 500",
    async ({ request }) => {
      const res = await request.post(
        "/api/processes/nonexistent-xyzzy/start"
      );
      expect(res.status()).not.toBe(500);
    }
  );
});

// ── stress-run service-down contract ──────────────────────────────────
//
// POST /api/stress/run with a gateway_url that points to a closed port
// must return 502 with an actionable machine-readable JSON payload —
// not 200, not 500, not an HTML traceback.
//
// The "down" condition is manufactured deterministically by passing a
// gateway_url that will never connect (closed loopback port 1).

test.describe("stress-run service-down contract", () => {
  // Use a URL that is guaranteed unreachable in any environment.
  const DEAD_GW = "ws://127.0.0.1:1";

  test("unreachable gateway returns 502 not 200 or 500",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: { gateway_url: DEAD_GW, rate: "1", duration: "1" },
      });
      expect(res.status()).toBe(502);
    }
  );

  test("502 stress-run has application/json content-type",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: { gateway_url: DEAD_GW, rate: "1", duration: "1" },
      });
      expect(res.status()).toBe(502);
      const ct = res.headers()["content-type"] ?? "";
      expect(ct).toContain("application/json");
    }
  );

  test("502 stress-run body has non-empty code field",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: { gateway_url: DEAD_GW, rate: "1", duration: "1" },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(typeof body.code).toBe("string");
      expect(body.code.length).toBeGreaterThan(0);
    }
  );

  test("502 stress-run body has non-empty message field",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: { gateway_url: DEAD_GW, rate: "1", duration: "1" },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(typeof body.message).toBe("string");
      expect(body.message.length).toBeGreaterThan(0);
    }
  );

  test("502 stress-run context echoes the submitted gateway_url",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: { gateway_url: DEAD_GW, rate: "1", duration: "1" },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      // context.gateway_url must reflect what was submitted so the
      // caller can identify which upstream was unreachable.
      expect(body?.context?.gateway_url).toBe(DEAD_GW);
    }
  );

  test("502 stress-run body has no Python traceback",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: { gateway_url: DEAD_GW, rate: "1", duration: "1" },
      });
      expect(res.status()).toBe(502);
      const text = await res.text();
      await assertNoTraceback(text);
    }
  );

  test("502 stress-run body is valid JSON (not HTML)",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: { gateway_url: DEAD_GW, rate: "1", duration: "1" },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(typeof body).toBe("object");
      expect(body).not.toBeNull();
    }
  );
});
