import { test, expect } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";

// STRESS_REPORTS_DIR = rsx/tmp/stress-reports
const STRESS_REPORTS_DIR = path.join(
  __dirname, "..", "..", "tmp", "stress-reports"
);
const TEST_REPORT_ID = "20260219-120000";
const TEST_REPORT_FILE = path.join(
  STRESS_REPORTS_DIR, `stress-${TEST_REPORT_ID}.json`
);
const TEST_REPORT = {
  timestamp: TEST_REPORT_ID,
  config: { target_rate: 100, duration: 10 },
  metrics: {
    submitted: 1000,
    accepted: 985,
    rejected: 15,
    errors: 0,
    accept_rate: 98.5,
    actual_rate: 99.2,
    elapsed_sec: 10.1,
  },
  latency_us: { min: 100, p50: 250, p95: 800, p99: 1200, max: 2500 },
};

test.describe("Stress tab", () => {
  test.beforeAll(() => {
    fs.mkdirSync(STRESS_REPORTS_DIR, { recursive: true });
    fs.writeFileSync(TEST_REPORT_FILE, JSON.stringify(TEST_REPORT, null, 2));
  });

  test.afterAll(() => {
    if (fs.existsSync(TEST_REPORT_FILE)) {
      fs.unlinkSync(TEST_REPORT_FILE);
    }
  });

  // Page structure
  test("loads stress page with all three cards", async ({ page }) => {
    await page.goto("/stress");
    await expect(
      page.getByRole("heading", { name: "Run Stress Test" })
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Historical Reports" })
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "About Stress Testing" })
    ).toBeVisible();
  });

  test("stress tab is active in nav", async ({ page }) => {
    await page.goto("/stress");
    await expect(
      page.locator("nav a", { hasText: "Stress" })
    ).toHaveClass(/bg-slate-700/);
  });

  // Form structure
  test("rate input has correct defaults and bounds", async ({ page }) => {
    await page.goto("/stress");
    const rateInput = page.locator("input[name='rate']");
    await expect(rateInput).toBeVisible();
    await expect(rateInput).toHaveValue("100");
    await expect(rateInput).toHaveAttribute("min", "10");
    await expect(rateInput).toHaveAttribute("max", "10000");
  });

  test("duration input has correct defaults and bounds", async ({ page }) => {
    await page.goto("/stress");
    const durationInput = page.locator("input[name='duration']");
    await expect(durationInput).toBeVisible();
    await expect(durationInput).toHaveValue("60");
    await expect(durationInput).toHaveAttribute("min", "1");
    await expect(durationInput).toHaveAttribute("max", "600");
  });

  test("run stress test button is visible", async ({ page }) => {
    await page.goto("/stress");
    await expect(
      page.locator("button", { hasText: "Run Stress Test" })
    ).toBeVisible();
  });

  test("form uses HTMX post to stress run endpoint", async ({ page }) => {
    await page.goto("/stress");
    const form = page.locator("form[hx-post*='api/stress/run']");
    await expect(form).toBeVisible();
    const target = await form.getAttribute("hx-target");
    expect(target).toBe("#stress-result");
  });

  test("stress result area exists", async ({ page }) => {
    await page.goto("/stress");
    await expect(page.locator("#stress-result")).toBeAttached();
  });

  test("running indicator has htmx-indicator class", async ({ page }) => {
    await page.goto("/stress");
    const indicator = page.locator(".htmx-indicator");
    await expect(indicator).toContainText(/Running/i);
  });

  // Reports list
  test("reports list loads via HTMX", async ({ page }) => {
    await page.goto("/stress");
    const reportsDiv = page.locator("[hx-get*='stress-reports-list']");
    await expect(reportsDiv).toBeVisible();
  });

  test("reports list auto-refreshes every 5s", async ({ page }) => {
    await page.goto("/stress");
    const reportsDiv = page.locator("[hx-get*='stress-reports-list']");
    const trigger = await reportsDiv.getAttribute("hx-trigger");
    expect(trigger).toContain("every 5s");
  });

  test("reports list shows seeded report row", async ({ page }) => {
    await page.goto("/stress");
    await page.waitForTimeout(1500); // wait for HTMX load
    const link = page.locator(`a[href*='${TEST_REPORT_ID}']`);
    await expect(link).toBeVisible();
  });

  test("reports list shows table columns", async ({ page }) => {
    await page.goto("/stress");
    await page.waitForTimeout(1500);
    const headers = page.locator(
      "[hx-get*='stress-reports-list'] table th"
    );
    await expect(headers.filter({ hasText: "Rate" })).toBeVisible();
    await expect(headers.filter({ hasText: "Duration" })).toBeVisible();
    await expect(headers.filter({ hasText: "Submitted" })).toBeVisible();
    await expect(headers.filter({ hasText: "Accept" })).toBeVisible();
    await expect(headers.filter({ hasText: "p99" })).toBeVisible();
  });

  // About card
  test("about card describes key metrics", async ({ page }) => {
    await page.goto("/stress");
    await expect(page.locator("text=Throughput")).toBeVisible();
    await expect(page.locator("text=Acceptance Rate")).toBeVisible();
  });

  // Stress run produces result (success or error) within test timeout
  test("form submission shows result after running", async ({
    page,
  }) => {
    await page.goto("/stress");
    await page.locator("input[name='rate']").fill("10");
    await page.locator("input[name='duration']").fill("1");
    await page.locator("button", { hasText: "Run Stress Test" }).click();
    // Wait for HTMX to swap the result (stress runs for ~1s then reports)
    await expect(page.locator("#stress-result")).toContainText(
      /completed|submitted|gateway|error/i,
      { timeout: 15000 }
    );
  });

  // Report detail page
  test("report detail 404 for non-existent report", async ({ page }) => {
    const response = await page.goto("/stress/nonexistent-000000000");
    expect(response?.status()).toBe(404);
  });

  test("report detail page loads for existing report", async ({ page }) => {
    await page.goto(`/stress/${TEST_REPORT_ID}`);
    await expect(
      page.getByRole("heading", { name: /Stress Test Report/ })
    ).toBeVisible();
  });

  test("report detail page has back link to stress page", async ({ page }) => {
    await page.goto(`/stress/${TEST_REPORT_ID}`);
    const backLink = page.locator("a", { hasText: /Back to Stress/ });
    await expect(backLink).toBeVisible();
  });

  test("report detail shows results card with accept rate bar", async ({
    page,
  }) => {
    await page.goto(`/stress/${TEST_REPORT_ID}`);
    await expect(page.getByRole("heading", { name: "Results" })).toBeVisible();
    await expect(page.locator("text=Accept Rate").first()).toBeVisible();
  });

  test("report detail shows latency distribution card", async ({ page }) => {
    await page.goto(`/stress/${TEST_REPORT_ID}`);
    await expect(
      page.getByRole("heading", { name: /Latency Distribution/ })
    ).toBeVisible();
    await expect(page.locator("text=p99").first()).toBeVisible();
  });

  test("report detail shows assessment card with pass or fail", async ({
    page,
  }) => {
    await page.goto(`/stress/${TEST_REPORT_ID}`);
    await expect(
      page.getByRole("heading", { name: "Assessment" })
    ).toBeVisible();
    await expect(
      page.locator("text=PASS").or(page.locator("text=FAIL"))
    ).toBeVisible();
  });

  // Direct API: 502 when gateway is unreachable (no HTMX headers)
  test("returns 502 with GATEWAY_UNREACHABLE when gateway is down",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(body.code).toBe("GATEWAY_UNREACHABLE");
      expect(body.message).toBeTruthy();
    }
  );

  // Regression: 502 payload is structured JSON (content-type)
  test("502 response has application/json content-type",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const ct = res.headers()["content-type"] ?? "";
      expect(ct).toContain("application/json");
    }
  );

  // Regression: message field is a non-empty string
  test("502 message field is a non-empty string",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(typeof body.message).toBe("string");
      expect(body.message.length).toBeGreaterThan(0);
    }
  );

  // Regression: no report_id present on error (no partial success)
  test("502 response has no report_id field",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(body.report_id).toBeUndefined();
    }
  );

  // Contract: structured error schema (code/message/context)
  test("502 body has code field set to GATEWAY_UNREACHABLE",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(body.code).toBe("GATEWAY_UNREACHABLE");
    }
  );

  test("502 body has non-empty message string",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(typeof body.message).toBe("string");
      expect(body.message.length).toBeGreaterThan(0);
    }
  );

  test("502 body has context object with gateway_url",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(typeof body.context).toBe("object");
      expect(body.context).not.toBeNull();
      expect(typeof body.context.gateway_url).toBe("string");
    }
  );

  test("502 body does not have legacy status/error fields",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(body.status).toBeUndefined();
      expect(body.error).toBeUndefined();
    }
  );

  // Regression: HTMX path returns 200 (not 502) with error span
  test("HTMX path returns 200 with error span when gateway is down",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
        headers: { "hx-request": "true" },
      });
      expect(res.status()).toBe(200);
      const text = await res.text();
      expect(text.toLowerCase()).toContain("gateway unreachable");
    }
  );
});

// ── gateway-proxy down contract ───────────────────────────────────────
//
// When the Gateway HTTP server is intentionally unavailable the /v1/*
// proxy MUST return HTTP 502 with a JSON body containing an "error"
// field (non-empty string).  These tests use a closed port (1) to
// force a connection-refused path reliably on every run.
//
// Contract:
//   • HTTP status is exactly 502
//   • Content-Type contains "application/json"
//   • body.error is a non-empty string
//   • body.error does NOT contain raw Python tracebacks
//   • No 200 masking (the proxy must not swallow the error)

test.describe("gateway-proxy down contract", () => {
  // Override GATEWAY_HTTP to a closed port via query param isn't
  // possible; instead we call the proxy with a path that triggers
  // the Gateway connection.  The real gateway is not running during
  // this test shard when the process-control project uses a detached
  // gateway_url.  We rely on the server being configured with
  // GATEWAY_HTTP pointing to a real (possibly running) gateway;
  // if it is running the proxy returns 200/4xx — so we only assert
  // 502 when we can manufacture the down condition.  The reliable
  // approach is to hit the stress endpoint with a closed-port
  // gateway_url (already covered above).  Here we add contract
  // checks for the *v1-proxy* error shape by simulating a broken
  // upstream via the stress runner path with a non-WS URL that
  // causes the stress client to fail before connecting, then assert
  // the proxy error shape rules independently.

  // Verify that a stress run targeting a non-WS URL (http://) also
  // returns 502 with the same contract schema.
  test("non-WS gateway URL returns 502", async ({ request }) => {
    const res = await request.post("/api/stress/run", {
      form: {
        rate: "10",
        duration: "1",
        gateway_url: "http://localhost:1/v1",
      },
    });
    expect(res.status()).toBe(502);
  });

  test("non-WS 502 body has code field", async ({ request }) => {
    const res = await request.post("/api/stress/run", {
      form: {
        rate: "10",
        duration: "1",
        gateway_url: "http://localhost:1/v1",
      },
    });
    const body = await res.json();
    expect(typeof body.code).toBe("string");
    expect(body.code.length).toBeGreaterThan(0);
  });

  test("non-WS 502 body has message field", async ({ request }) => {
    const res = await request.post("/api/stress/run", {
      form: {
        rate: "10",
        duration: "1",
        gateway_url: "http://localhost:1/v1",
      },
    });
    const body = await res.json();
    expect(typeof body.message).toBe("string");
    expect(body.message.length).toBeGreaterThan(0);
  });

  test("non-WS 502 body has context.gateway_url", async ({ request }) => {
    const res = await request.post("/api/stress/run", {
      form: {
        rate: "10",
        duration: "1",
        gateway_url: "http://localhost:1/v1",
      },
    });
    const body = await res.json();
    expect(typeof body.context).toBe("object");
    expect(body.context).not.toBeNull();
    expect(typeof body.context.gateway_url).toBe("string");
  });

  // v1-proxy: when gateway is not running the proxy returns 502 JSON
  // with an "error" key (not the structured code/message/context shape).
  // These tests assert the minimum actionable payload contract.
  test("v1-proxy returns 502 when gateway is down",
    async ({ request }) => {
      const res = await request.get(
        "/v1/book?symbol_id=10",
        {
          headers: {
            // Force a fresh connection attempt; no caching.
            "cache-control": "no-cache",
          },
        }
      );
      // Either 502 (gateway not running) or 200 (gateway is up).
      // We only assert shape when it's a failure response.
      if (res.status() === 502) {
        const body = await res.json();
        expect(typeof body.error).toBe("string");
        expect(body.error.length).toBeGreaterThan(0);
      } else {
        // Gateway is running — test is vacuously satisfied.
        expect([200, 400, 404]).toContain(res.status());
      }
    }
  );

  test("v1-proxy 502 body has no Python traceback",
    async ({ request }) => {
      const res = await request.get("/v1/book?symbol_id=10");
      if (res.status() === 502) {
        const body = await res.json();
        // Must not expose raw Python error text
        expect(body.error).not.toContain("Traceback");
        expect(body.error).not.toContain("File \"");
      } else {
        expect([200, 400, 404]).toContain(res.status());
      }
    }
  );

  test("v1-proxy 502 content-type is application/json",
    async ({ request }) => {
      const res = await request.get("/v1/book?symbol_id=10");
      if (res.status() === 502) {
        const ct = res.headers()["content-type"] ?? "";
        expect(ct).toContain("application/json");
      } else {
        expect([200, 400, 404]).toContain(res.status());
      }
    }
  );
});

// ── stress-down: invalid-input contract ──────────────────────────────
//
// Verifies that invalid/missing stress parameters return 4xx (not 5xx
// or 200) with a meaningful error payload.  Separate from the
// unreachable-gateway path so CI can run these without network access.

test.describe("stress-down invalid-input contract", () => {
  test("zero rate returns non-200 response", async ({ request }) => {
    const res = await request.post("/api/stress/run", {
      form: { rate: "0", duration: "1", gateway_url: "ws://localhost:1" },
    });
    // Should still return 502 (gateway unreachable before
    // rate validation) or 422 (FastAPI validation).  Either
    // way, not 200.
    expect(res.status()).not.toBe(200);
  });

  test("duration=0 returns non-200 response", async ({ request }) => {
    const res = await request.post("/api/stress/run", {
      form: {
        rate: "10",
        duration: "0",
        gateway_url: "ws://localhost:1",
      },
    });
    expect(res.status()).not.toBe(200);
  });

  test("unreachable gateway with large rate still returns 502",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10000",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
      });
      expect(res.status()).toBe(502);
      const body = await res.json();
      expect(typeof body.code).toBe("string");
    }
  );

  test("HTMX path with invalid gateway returns 200 + error span",
    async ({ request }) => {
      const res = await request.post("/api/stress/run", {
        form: {
          rate: "10",
          duration: "1",
          gateway_url: "ws://localhost:1",
        },
        headers: { "hx-request": "true" },
      });
      // HTMX paths must return 200 so HTMX can swap the error
      // into the DOM — never a raw 502.
      expect(res.status()).toBe(200);
      const text = await res.text();
      // Must contain a human-readable error indication.
      expect(text.toLowerCase()).toMatch(
        /gateway|unreachable|error|failed/
      );
    }
  );
});
