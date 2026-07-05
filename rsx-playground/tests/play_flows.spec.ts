import { test, expect, APIRequestContext } from "@playwright/test";
import { waitForHTMX } from "./test_helpers";

// ── Layer 4: ORDER FLOW ─────────────────────────────────────
// End-to-end order lifecycle through the real cluster. The audit
// found a FILLED order whose Lifecycle Trace was stuck "pending"
// forever (#6), a Custom Order form that could not place a resting
// GTC without overflow/reject (#4/#5), and rejects shown as raw
// numeric codes (#15). A render-only suite saw none of it.

async function clusterUp(req: APIRequestContext): Promise<number> {
  // Retry: a single transient /healthz blip must not false-skip.
  for (let i = 0; i < 3; i++) {
    try {
      const r = await req.get("/healthz", { timeout: 4000 });
      if (r.ok()) return (await r.json()).processes_running ?? 0;
    } catch { /* retry */ }
    await new Promise((res) => setTimeout(res, 500));
  }
  return 0;
}

const NUMERIC_REJECT = /reason\s*=?\s*\d/i;  // the bare "reason=4" bug

test.describe("Order flows", () => {
  test.beforeEach(async ({ request }) => {
    test.skip((await clusterUp(request)) < 6, "cluster not up (need 6+)");
  });

  // ── FINDING #6: market order fills AND its Lifecycle Trace reaches
  // filled/done (not stuck "pending awaiting gateway" forever).
  test("market order fills and its trace reaches done", async ({
    page, request,
  }) => {
    const resp = await request.post("/api/orders/test", {
      form: {
        symbol_id: "10", side: "buy", price: "0", qty: "10",
        tif: "IOC", user_id: "1",
      },
    });
    const result = await resp.text();
    expect(result).toMatch(/filled/i);
    expect(result).not.toMatch(NUMERIC_REJECT);
    const cid = result.match(/pg[0-9a-f]+/)?.[0];
    expect(cid, "no cid in order result").toBeTruthy();

    // Trace that cid via the Order Lifecycle Trace UI.
    await page.goto("/orders");
    await page.locator("#trace-oid").fill(cid!);
    await page.locator("button", { hasText: "Trace" }).click();
    await waitForHTMX(page, 2000);
    const trace = page.locator("#trace-result");
    // Must advance past submitted/pending to a terminal state.
    await expect(trace).toContainText(/filled|done/i, { timeout: 5000 });
    const t = (await trace.innerText()).toLowerCase();
    expect(t).not.toContain("awaiting gateway");
  });

  // ── FINDING #4/#5: a resting GTC with the (now valid) defaults is
  // accepted (resting/filled), NOT rejected with an overflow.
  test("resting GTC is accepted, not rejected/overflow", async ({
    request,
  }) => {
    const resp = await request.post("/api/orders/test", {
      form: {
        symbol_id: "10", side: "buy", price: "0.05", qty: "10",
        tif: "GTC", user_id: "1",
      },
    });
    const result = await resp.text();
    expect(result).toMatch(/resting|filled|accepted/i);
    expect(result).not.toMatch(/rejected|overflow/i);
    expect(result).not.toMatch(NUMERIC_REJECT);
  });

  // ── FINDING #4/#5: the Custom Order form's PREFILLED defaults are
  // valid (lot-aligned qty, no notional overflow) — submitting the
  // form as-is must not reject.
  test("Custom Order form defaults submit without rejection", async ({
    page,
  }) => {
    await page.goto("/orders");
    await page.locator("summary", { hasText: "Custom Order" }).click();
    await page.locator(
      "form[hx-post='./api/orders/test'] button[type='submit']").click();
    const out = page.locator("#order-result");
    // The prefilled defaults must produce a real outcome, never a
    // lot-alignment/overflow reject (#4/#5).
    await expect(out).toContainText(/filled|resting|accepted/i,
      { timeout: 5000 });
    const txt = await out.innerText();
    expect(txt).not.toMatch(/not aligned|overflow/i);
    expect(txt).not.toMatch(NUMERIC_REJECT);
  });

  // ── FINDING #15: rejections render as human words, never "reason=N".
  test("rejects render as words, not numeric codes", async ({
    request,
  }) => {
    // qty=1 is not aligned to the PENGU lot (100000) → a deterministic
    // reject. It must read as a human string.
    const resp = await request.post("/api/orders/test", {
      form: {
        symbol_id: "10", side: "buy", price: "0.05", qty: "1",
        tif: "GTC", user_id: "1",
      },
    });
    const result = await resp.text();
    expect(result).toMatch(/not aligned|lot|invalid|reject/i);
    expect(result, `reject leaked a bare numeric code: ${result}`)
      .not.toMatch(NUMERIC_REJECT);
  });
});
