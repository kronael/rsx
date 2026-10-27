import { test, expect, type APIRequestContext } from "@playwright/test";
import { minOrderQty } from "./test_helpers";

// Functional only: what the gateway counts and which card shows it. Latency
// belongs to the Criterion benches and their gate.

type Reject = { code: number; name: string; count: number };
type Gateway = {
  name: string;
  error: string;
  state: string;
  circuit: string;
  pending: { used: number; cap: number };
  counters: Record<string, number>;
  rejects: Reject[];
};

async function gateways(request: APIRequestContext) {
  const res = await request.get("/api/gateway/metrics");
  expect(res.ok()).toBeTruthy();
  const body = (await res.json()) as { gateways: Gateway[] };
  return body.gateways.filter((gateway) => !gateway.error);
}

async function fokBuyBelowEveryAsk(request: APIRequestContext) {
  const res = await request.post("/api/orders/test", {
    form: {
      symbol_id: "10", side: "buy", price: "0.001",
      // sized to the minimum at its own limit price: a smaller order is
      // ORDER_TOO_SMALL, not the FILL_OR_KILL reject this counts
      qty: await minOrderQty(request, 10, 0.001),
      tif: "FOK", user_id: "1",
    },
  });
  expect(res.ok(), `${res.status()} ${await res.text()}`).toBeTruthy();
}

function sum(rows: Gateway[], pick: (gateway: Gateway) => number) {
  return rows.reduce((total, gateway) => total + pick(gateway), 0);
}

test.describe("Gateway tab", () => {
  /**
   * A fresh page load -> every card is present and the nav tab is active.
   */
  test("loads with its cards", async ({ page }) => {
    await page.goto("/gateway");
    await expect(page.locator("nav a", { hasText: "Gateway" }))
      .toHaveClass(/bg-slate-700/);
    for (const title of [
      "Gateways", "Rejects by Reason", "Duplicate Client Order Id",
    ])
      await expect(page.getByRole("heading", { name: title })).toBeVisible();
  });

  /**
   * The gateways card polls and lists gw-0 with its connections, forwarded
   * orders, rejects, pending set and a closed circuit.
   */
  test("gateways card shows the per-gateway counters", async ({ page }) => {
    await page.goto("/gateway");
    const card = page.locator("div[hx-get='./x/gateway-counters']");
    const row = card.locator("tbody tr").first();
    await expect(row).toContainText("gw-0");
    await expect(row).toContainText(/\d+ \/ \d+/);
    await expect(row).toContainText("closed");
    for (const head of ["Connections", "Forwarded", "Rejects", "Pending", "Circuit"])
      await expect(card.getByRole("columnheader", { name: head, exact: true })).toBeVisible();
    expect(await card.getAttribute("hx-trigger")).toContain("every 2s");
  });

  /**
   * The JSON behind the card: spec 18 reason names with their wire codes,
   * the pending set within its cap and the circuit as a word.
   */
  test("the metrics endpoint names every reject reason", async ({ request }) => {
    const [gateway] = await gateways(request);
    expect(gateway.rejects.map((reject) => reject.code))
      .toEqual(Array.from({ length: 15 }, (_, code) => code));
    const names = gateway.rejects.map((reject) => reject.name);
    expect(names).toContain("DUPLICATE_ORDER_ID");
    expect(names).toContain("FILL_OR_KILL");
    expect(names).toContain("ORDER_TOO_SMALL");
    expect(gateway.pending.used).toBeLessThanOrEqual(gateway.pending.cap);
    expect(["closed", "open", "half-open"]).toContain(gateway.circuit);
  });

  /**
   * An order through the gateway -> its forwarded counter rises.
   */
  test("an order raises the forwarded counter", async ({ request }) => {
    const before = sum(await gateways(request), (gateway) => gateway.counters.orders_processed);
    await fokBuyBelowEveryAsk(request);
    await expect.poll(async () =>
      sum(await gateways(request), (gateway) => gateway.counters.orders_processed),
      { timeout: 8000 }).toBeGreaterThan(before);
  });

  /**
   * A FOK buy priced below every ask cannot fill -> the engine fails it and
   * the gateway counts FILL_OR_KILL, which the rejects card lists.
   */
  test("a failed order is counted under its reason", async ({ page, request }) => {
    const count = async () => sum(await gateways(request), (gateway) =>
      gateway.rejects.find((reject) => reject.name === "FILL_OR_KILL")!.count);
    const before = await count();
    await fokBuyBelowEveryAsk(request);
    await expect.poll(count, { timeout: 8000 }).toBeGreaterThan(before);

    await page.goto("/gateway");
    const card = page.locator("div[hx-get='./x/gateway-rejects']");
    const row = card.locator("tr", { hasText: "FILL_OR_KILL" });
    await expect(row).toContainText("13");
    await expect(row.locator("td").last()).not.toHaveText("0");
    await expect(card.locator("tr", { hasText: "DUPLICATE_ORDER_ID" })).toContainText("3");
  });

  /**
   * The same client order id sent twice over the real gateway -> the second
   * send is answered with the duplicate order id reason.
   */
  test("the duplicate demo shows the gateway answering a repeated cid", async ({ page }) => {
    await page.goto("/gateway");
    await page.locator("#duplicate-run").click();
    await expect(page.locator("#duplicate-outcome"))
      .toContainText("answered: duplicate order id", { timeout: 15_000 });
    const rows = page.locator("#duplicate-result tbody tr");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(1)).toContainText("rejected: DUPLICATE_ORDER_ID");
    await expect(rows.nth(0)).not.toContainText("DUPLICATE_ORDER_ID");
  });
});
