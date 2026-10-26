import { test, expect, APIRequestContext } from "@playwright/test";
import { waitForHTMX, minOrderQty } from "./test_helpers";

// ── Layer 4: ORDER FLOW ─────────────────────────────────────
// End-to-end order lifecycle through the real cluster.

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

const NUMERIC_REJECT = /reason\s*=?\s*\d/i;

test.describe("Order flows", () => {
  test.beforeEach(async ({ request }) => {
    test.skip((await clusterUp(request)) < 6, "cluster not up (need 6+)");
  });

  test("order trace reaches a resolved state", async ({
    page, request,
  }) => {
    const resp = await request.post("/api/orders/test", {
      form: {
        symbol_id: "10", side: "buy", order_type: "limit",
        price: "0.049", qty: await minOrderQty(request, 10, 0.049),
        tif: "GTC", user_id: "1",
      },
    });
    const result = await resp.text();
    expect(result).toMatch(/resting|filled|accepted/i);
    expect(result).not.toMatch(NUMERIC_REJECT);
    const cid = result.match(/pg[0-9a-f]+/)?.[0];
    expect(cid, "no cid in order result").toBeTruthy();

    // Trace that cid via the Order Lifecycle Trace UI.
    await page.goto("/orders");
    await page.locator("#trace-oid").fill(cid!);
    await page.locator("button", { hasText: "Trace" }).click();
    await waitForHTMX(page, 2000);
    const trace = page.locator("#trace-result");
    // Must advance past submitted/pending to a resolved state.
    await expect(trace).toContainText(
      /resting|filled|done|accepted/i,
      { timeout: 5000 },
    );
    const t = (await trace.innerText()).toLowerCase();
    expect(t).not.toContain("awaiting gateway");
  });

  test("resting GTC is accepted, not rejected/overflow", async ({
    request,
  }) => {
    const resp = await request.post("/api/orders/test", {
      form: {
        symbol_id: "10", side: "buy", price: "0.049",
        qty: await minOrderQty(request, 10, 0.049),
        tif: "GTC", user_id: "1",
      },
    });
    const result = await resp.text();
    expect(result).toMatch(/resting|filled|accepted/i);
    expect(result).not.toMatch(/rejected|overflow/i);
    expect(result).not.toMatch(NUMERIC_REJECT);
  });

  // The prefilled defaults are lot-aligned and fit the notional range.
  test("Custom Order form defaults submit without rejection", async ({
    page,
  }) => {
    await page.goto("/orders");
    await page.locator("summary", { hasText: "Custom Order" }).click();
    await page.locator(
      "form[hx-post='./api/orders/test'] button[type='submit']").click();
    const out = page.locator("#order-result");
    // The prefilled defaults must produce a real outcome.
    await expect(out).toContainText(/filled|resting|accepted/i,
      { timeout: 5000 });
    const txt = await out.innerText();
    expect(txt).not.toMatch(/not aligned|overflow/i);
    expect(txt).not.toMatch(NUMERIC_REJECT);
  });

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
