import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import { waitForHTMX, minOrderQty } from "./test_helpers";

// Functional only: what the matching engine did with each order and which
// card shows it. Latency belongs to the Criterion benches and their gate.

type MatchingCounters = { mes: { name: string; counters: Record<string, number> }[] };

const ASK_PRICE = "0.9";

async function counters(request: APIRequestContext) {
  const res = await request.get("/api/matching/counters");
  expect(res.ok()).toBeTruthy();
  const body = (await res.json()) as MatchingCounters;
  return body.mes.find((me) => me.name === "me-pengu")!.counters;
}

async function submit(page: Page, order: {
  side: "buy" | "sell";
  price: string;
  tif: "GTC" | "IOC" | "FOK";
  userId: string;
  postOnly?: boolean;
}) {
  await page.locator("input[type='radio'][name='side']" + `[value='${order.side}']`)
    .check({ force: true });
  await page.locator("input[name='price']").fill(order.price);
  await page.locator("input[name='qty']")
    .fill(await minOrderQty(page.request, 10, Number(order.price)));
  await page.locator("input[type='radio'][name='tif']" + `[value='${order.tif}']`)
    .check({ force: true });
  await page.locator("input[name='user_id']").fill(order.userId);
  if (order.postOnly)
    await page.locator("input[name='post_only']").check({ force: true });
  else
    await page.locator("input[name='post_only']").uncheck({ force: true });
  await page.locator("form[hx-post='./api/orders/test'] button[type='submit']").click();
}

async function restingCid(page: Page) {
  const result = page.locator("#order-result");
  await expect(result).toContainText(/order \w+ resting/, { timeout: 8000 });
  const text = (await result.textContent()) ?? "";
  return /order (\w+) resting/.exec(text)![1];
}

test.describe("Matching tab", () => {
  /**
   * A fresh page load -> every card is present and the nav tab is active.
   */
  test("loads with its cards", async ({ page }) => {
    await page.goto("/matching");
    await expect(page.locator("nav a", { hasText: "Matching" }))
      .toHaveClass(/bg-slate-700/);
    for (const title of [
      "Matching Engines", "Order Entry", "Recent Orders",
      "Duplicate Order Id", "Recovery", "Rejects and Engine Cancels",
    ])
      await expect(page.getByRole("heading", { name: title })).toBeVisible();
  });

  /**
   * The engines card polls and lists the PENGU engine with its counters.
   */
  test("engines card shows the per-ME counters", async ({ page }) => {
    await page.goto("/matching");
    const card = page.locator("div[hx-get='./x/matching-counters']");
    await expect(card.locator("tbody tr").first()).toContainText("me-pengu");
    await expect(card.locator("th", { hasText: "Orders" })).toBeVisible();
    await expect(card.getByRole("columnheader", { name: "Dup", exact: true }))
      .toBeVisible();
    expect(await card.getAttribute("hx-trigger")).toContain("every 2s");
  });

  /**
   * A FOK buy priced below every ask cannot fill -> the engine rejects it
   * with the wire reason, its FOK counter rises and the WAL feed lists it.
   */
  test("a fill-or-kill that cannot fill is rejected", async ({ page, request }) => {
    const before = (await counters(request)).rejects_fill_or_kill;
    await page.goto("/matching");
    await submit(page, { side: "buy", price: "0.001", tif: "FOK", userId: "1" });
    await expect(page.locator("#order-result"))
      .toContainText("rejected: fill-or-kill not fillable", { timeout: 8000 });
    await expect.poll(async () => (await counters(request)).rejects_fill_or_kill)
      .toBeGreaterThan(before);
    await expect(page.locator("div[hx-get='./x/matching-rejects']"))
      .toContainText("fill-or-kill not fillable", { timeout: 8000 });
  });

  /**
   * A resting ask, then a post-only buy at the same price -> the buy is
   * cancelled instead of crossing; the Cancel button then cancels the ask at
   * the engine, which the gateway confirms with a cancelled frame.
   */
  test("post-only cancels on a cross and Cancel removes a resting order",
    async ({ page }) => {
      test.setTimeout(45_000);
      await page.goto("/matching");
      await submit(page, { side: "sell", price: ASK_PRICE, tif: "GTC", userId: "2" });
      const cid = await restingCid(page);

      await submit(page, {
        side: "buy", price: ASK_PRICE, tif: "GTC", userId: "1", postOnly: true,
      });
      await expect(page.locator("#order-result")).toContainText("cancelled", { timeout: 8000 });
      const feed = page.locator("div[hx-get='./x/matching-rejects']");
      await expect(feed).toContainText("post_only_reject", { timeout: 8000 });

      const recent = page.locator("div[hx-get='./x/recent-orders']");
      const cancel = recent.locator(`button[hx-post$='/${cid}/cancel']`);
      await expect(cancel).toBeVisible({ timeout: 5000 });
      await cancel.click();
      await expect(page.locator("#order-result")).toContainText(`${cid} cancelled`, { timeout: 8000 });
      await expect(recent.locator("tr", { hasText: cid })).toContainText("cancelled", { timeout: 5000 });
    });

  /**
   * The same order id cast twice to a scratch engine -> the second copy is
   * answered with ORDER_FAILED, duplicate order id.
   */
  test("the duplicate demo shows the engine rejecting a repeated id", async ({ page }) => {
    await page.goto("/matching");
    await page.locator("#duplicate-run").click();
    await expect(page.locator("#duplicate-outcome"))
      .toContainText("rejected: duplicate order id", { timeout: 20_000 });
    const rows = page.locator("#duplicate-result tbody tr");
    await expect(rows.nth(0)).toContainText("order_accepted");
    await expect(rows.last()).toContainText("order_failed");
  });

  /**
   * Crash the engine -> it returns with everything it had flushed (WAL tip
   * at least the pre-crash flushed seq), a checkpoint not older than before
   * and the same config version.
   */
  test("crash and recover keeps the WAL tip and checkpoint", async ({ page }) => {
    test.setTimeout(60_000);
    await page.goto("/matching");
    const recovery = page.locator("div[hx-get='./x/matching-recovery']");
    await expect(recovery.locator("tbody tr").first()).toContainText("me-pengu");
    await expect(recovery.locator("th", { hasText: "Checkpoint" })).toBeVisible();
    page.once("dialog", (dialog) => dialog.accept());
    await recovery.locator("button", { hasText: "Crash and recover" }).first().click();
    await expect(page.locator("#recovery-outcome")).toContainText("is back", { timeout: 45_000 });
    const rows = page.locator("#recovery-result tbody tr");
    const before = (await rows.nth(0).locator("td").allTextContents()).map(Number);
    const after = (await rows.nth(1).locator("td").allTextContents()).map(Number);
    const [tip, flushed, checkpoint, version] = [1, 2, 3, 4];
    expect(after[tip]).toBeGreaterThanOrEqual(before[flushed]);
    expect(after[checkpoint]).toBeGreaterThanOrEqual(before[checkpoint]);
    expect(after[version]).toBe(before[version]);
    await waitForHTMX(page);
    await expect(recovery.locator("tbody tr").first()).toContainText("me-pengu");
  });
});
